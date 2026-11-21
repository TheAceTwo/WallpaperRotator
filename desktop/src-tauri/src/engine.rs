//! Fetch → pick → download → apply, plus the background schedule.

use crate::config::{self, Config, State};
use crate::schedule::{self, QueuedPhoto, QUEUE_LEN};
use immich_core::{Asset, ImmichClient, ShareLink};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

/// How long a fetched album listing is reused before asking the server again.
const LIST_CACHE: Duration = Duration::from_secs(3 * 60 * 60);

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Unix seconds of the last successful change.
    pub last_changed: Option<u64>,
    pub last_error: Option<String>,
    pub photo_count: Option<usize>,
    pub lock_screen_note: Option<String>,
}

struct ListCache {
    share_url: String,
    fetched: Instant,
    assets: Vec<Asset>,
}

pub struct Engine {
    pub config: Mutex<Config>,
    state: Mutex<State>,
    status: Mutex<Status>,
    cache: Mutex<Option<ListCache>>,
    /// The link (main or backup) that worked most recently; tried first.
    last_good_link: Mutex<Option<String>>,
    /// Serialises rotations so the timer and "Next" can't overlap.
    busy: tokio::sync::Mutex<()>,
    /// Only one cache refill at a time.
    refilling: tokio::sync::Mutex<()>,
    /// Wakes the scheduler (settings changed / resumed).
    pub wake: Notify,
}

impl Engine {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            config: Mutex::new(config::load_config(app)),
            state: Mutex::new(config::load_state(app)),
            status: Mutex::new(Status::default()),
            cache: Mutex::new(None),
            last_good_link: Mutex::new(None),
            busy: tokio::sync::Mutex::new(()),
            refilling: tokio::sync::Mutex::new(()),
            wake: Notify::new(),
        }
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn invalidate_cache(&self) {
        *self.cache.lock().unwrap() = None;
    }

    /// The share link and, if enabled, the backup link, with whichever
    /// worked last time first (so a link that's unreachable on this network
    /// doesn't make every change wait for it to time out).
    fn links(&self, cfg: &Config) -> Result<Vec<(String, ImmichClient)>, String> {
        let last_good = self.last_good_link.lock().unwrap().clone();
        let backup = cfg.use_backup_link.then_some(cfg.backup_share_url.as_str());
        schedule::link_order(&cfg.share_url, backup, last_good.as_deref())
            .into_iter()
            .map(|u| {
                let link = ShareLink::parse(&u).map_err(|e| e.to_string())?;
                Ok((u, ImmichClient::new(link).map_err(|e| e.to_string())?))
            })
            .collect()
    }

    /// Combine per-link errors into one readable message.
    fn link_errors(cfg: &Config, errors: Vec<(String, String)>) -> String {
        if errors.len() == 1 {
            return errors.into_iter().next().map(|(_, e)| e).unwrap_or_default();
        }
        errors
            .into_iter()
            .map(|(u, e)| {
                let which = if u == cfg.share_url.trim() { "Main link" } else { "Backup link" };
                format!("{which}: {e}")
            })
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Album listing (cached), trying each link in turn.
    async fn assets(&self, cfg: &Config) -> Result<Vec<Asset>, String> {
        if let Some(c) = self.cache.lock().unwrap().as_ref() {
            if c.share_url == cfg.share_url && c.fetched.elapsed() < LIST_CACHE {
                return Ok(c.assets.clone());
            }
        }
        let mut errors = Vec::new();
        for (url, client) in self.links(cfg)? {
            match client.list_assets().await {
                Ok(assets) => {
                    *self.last_good_link.lock().unwrap() = Some(url);
                    // Both links show the same album, so cache by the main link.
                    *self.cache.lock().unwrap() = Some(ListCache {
                        share_url: cfg.share_url.clone(),
                        fetched: Instant::now(),
                        assets: assets.clone(),
                    });
                    return Ok(assets);
                }
                Err(e) => errors.push((url, e.to_string())),
            }
        }
        Err(Self::link_errors(cfg, errors))
    }

    /// Download a photo, trying each link in turn.
    async fn download(
        &self,
        cfg: &Config,
        asset: &Asset,
        dir: &Path,
        prefix: &str,
        stamp: u64,
    ) -> Result<PathBuf, String> {
        let mut errors = Vec::new();
        for (url, client) in self.links(cfg)? {
            match download_to(&client, asset, cfg.prefer_original, dir, prefix, stamp).await {
                Ok(path) => {
                    *self.last_good_link.lock().unwrap() = Some(url);
                    return Ok(path);
                }
                Err(e) => errors.push((url, e)),
            }
        }
        Err(Self::link_errors(cfg, errors))
    }

    /// Change the wallpaper now. Records the outcome in `status`.
    pub async fn rotate(&self, app: &AppHandle) -> Result<(), String> {
        let result = {
            let _guard = self.busy.lock().await;
            self.rotate_inner(app).await
        };
        {
            let mut st = self.status.lock().unwrap();
            match &result {
                Ok(()) => {
                    st.last_changed = Some(now_secs());
                    st.last_error = None;
                }
                Err(e) => st.last_error = Some(e.clone()),
            }
        }
        if result.is_ok() {
            let state = {
                let mut st = self.state.lock().unwrap();
                st.last_changed = Some(now_secs());
                st.clone()
            };
            let _ = config::save_state(app, &state);
        }
        let _ = app.emit("status", self.status());
        // Top the cache back up in the background (needs network; may fail).
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move { engine(&app2).refill_queue(&app2).await });
        result
    }

    async fn rotate_inner(&self, app: &AppHandle) -> Result<(), String> {
        let cfg = self.config();
        if cfg.share_url.trim().is_empty() {
            return Err("Add your Immich share link in Settings first.".into());
        }
        let dir = config::wallpaper_dir(app);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = now_millis();

        let want_lock = cfg.set_lock_screen && wallsetter::SUPPORTS_SEPARATE_LOCK_SCREEN;
        let need = if want_lock && cfg.separate_lock_image { 2 } else { 1 };

        // 1. Use photos downloaded ahead of time (instant, works offline).
        let queued = {
            let mut st = self.state.lock().unwrap();
            self.prune_locked(&mut st, &cfg);
            schedule::take(&mut st.queue, need)
        };
        let mut originals: Vec<(String, PathBuf)> = Vec::new();
        for (i, q) in queued.into_iter().enumerate() {
            // Move it out of the cache folder so normal cleanup manages it.
            let ext = q.path.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_string();
            let dest = dir.join(format!("{}-orig-{stamp}.{ext}", if i == 0 { "desktop" } else { "lock" }));
            if std::fs::rename(&q.path, &dest).is_ok() {
                originals.push((q.asset_id, dest));
            }
        }

        // 2. Download whatever is still missing.
        if originals.len() < need {
            let assets = self.assets(&cfg).await?;
            self.status.lock().unwrap().photo_count = Some(assets.iter().filter(|a| a.is_image).count());
            while originals.len() < need {
                let asset = {
                    let mut st = self.state.lock().unwrap();
                    immich_core::pick(&assets, cfg.orientation, &mut st.history, &mut rand::thread_rng()).cloned()
                }
                .ok_or("The shared album has no photos.")?;
                let prefix = if originals.is_empty() { "desktop-orig" } else { "lock-orig" };
                let path = self.download(&cfg, &asset, &dir, prefix, stamp).await?;
                originals.push((asset.id, path));
            }
        }

        let desktop_orig = originals[0].1.clone();
        let lock_orig = if !want_lock {
            None
        } else if let Some((id, p)) = originals.get(1) {
            // Separate lock photo, unless it happens to be the same one.
            Some(if *id == originals[0].0 { desktop_orig.clone() } else { p.clone() })
        } else {
            Some(desktop_orig.clone())
        };
        {
            let mut st = self.state.lock().unwrap();
            st.current_desktop_original = Some(desktop_orig.clone());
            st.current_lock_original = lock_orig.clone();
        }
        self.apply_originals(app, &cfg, &dir, desktop_orig, lock_orig, stamp).await
    }

    /// Drop cached photos that don't match the current settings.
    fn prune_locked(&self, st: &mut State, cfg: &Config) {
        for p in schedule::prune_queue(&mut st.queue, &cfg.share_url, cfg.orientation, Path::exists) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// After settings change: drop stale cached photos and download fresh ones.
    pub fn settings_changed(&self, app: &AppHandle) {
        let cfg = self.config();
        let state = {
            let mut st = self.state.lock().unwrap();
            self.prune_locked(&mut st, &cfg);
            st.clone()
        };
        let _ = config::save_state(app, &state);
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move { engine(&app2).refill_queue(&app2).await });
    }

    /// Download photos until QUEUE_LEN are waiting. Quietly stops on errors
    /// (e.g. offline); the next change tries again.
    pub async fn refill_queue(&self, app: &AppHandle) {
        let Ok(_guard) = self.refilling.try_lock() else { return };
        let cfg = self.config();
        if cfg.share_url.trim().is_empty() {
            return;
        }
        let qdir = config::wallpaper_dir(app).join("queue");
        if std::fs::create_dir_all(&qdir).is_err() {
            return;
        }
        loop {
            let missing = {
                let mut st = self.state.lock().unwrap();
                self.prune_locked(&mut st, &cfg);
                QUEUE_LEN.saturating_sub(st.queue.len())
            };
            if missing == 0 {
                break;
            }
            let Ok(assets) = self.assets(&cfg).await else { break };
            let asset = {
                let mut st = self.state.lock().unwrap();
                immich_core::pick(&assets, cfg.orientation, &mut st.history, &mut rand::thread_rng()).cloned()
            };
            let Some(asset) = asset else { break };
            let Ok(path) = self.download(&cfg, &asset, &qdir, "q", now_millis()).await else {
                break;
            };
            let state = {
                let mut st = self.state.lock().unwrap();
                st.queue.push(QueuedPhoto {
                    asset_id: asset.id,
                    path,
                    share_url: cfg.share_url.clone(),
                    orientation: cfg.orientation,
                });
                st.clone()
            };
            let _ = config::save_state(app, &state);
        }
        // Remove stray files no longer referenced (e.g. after a crash).
        let keep: Vec<PathBuf> = self.state.lock().unwrap().queue.iter().map(|q| q.path.clone()).collect();
        if let Ok(entries) = std::fs::read_dir(&qdir) {
            for e in entries.flatten() {
                if !keep.contains(&e.path()) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    /// Re-apply the current photos with the current dim setting (no download).
    pub async fn reapply(&self, app: &AppHandle) -> Result<(), String> {
        let _guard = self.busy.lock().await;
        let cfg = self.config();
        let (desktop_orig, lock_orig) = {
            let st = self.state.lock().unwrap();
            (st.current_desktop_original.clone(), st.current_lock_original.clone())
        };
        let desktop_orig = desktop_orig
            .filter(|p| p.exists())
            .ok_or("No wallpaper has been set yet. It will be dimmed when the next photo is applied.")?;
        let want_lock = cfg.set_lock_screen && wallsetter::SUPPORTS_SEPARATE_LOCK_SCREEN;
        let lock_orig = lock_orig.filter(|p| want_lock && p.exists());
        let dir = config::wallpaper_dir(app);
        let result = self.apply_originals(app, &cfg, &dir, desktop_orig, lock_orig, now_millis()).await;
        if let Err(e) = &result {
            self.status.lock().unwrap().last_error = Some(e.clone());
        }
        let _ = app.emit("status", self.status());
        result
    }

    /// Dim (if enabled) and apply the given original files, then tidy up.
    async fn apply_originals(
        &self,
        app: &AppHandle,
        cfg: &Config,
        dir: &Path,
        desktop_orig: PathBuf,
        lock_orig: Option<PathBuf>,
        stamp: u64,
    ) -> Result<(), String> {
        let fit = screen_fit(app);
        let main_fx = cfg.main_effects();
        let other_fx = cfg.other_effects();
        let lock_fx = cfg.lock_effects();

        let desktop_file = processed(&desktop_orig, dir, "desktop", stamp, main_fx, fit).await?;
        // Other monitors only need their own file when their effects differ
        // and there actually is another monitor.
        let multi = monitor_count(app).await > 1;
        let other_file = if multi && other_fx != main_fx {
            processed(&desktop_orig, dir, "other", stamp, other_fx, fit).await?
        } else {
            desktop_file.clone()
        };
        let lock_file = match lock_orig.clone() {
            // Same photo and same effects as the desktop: reuse the file.
            Some(l) if l == desktop_orig && lock_fx == main_fx => Some(desktop_file.clone()),
            Some(l) => Some(processed(&l, dir, "lock", stamp, lock_fx, fit).await?),
            None => None,
        };

        let prev_desktop = self.state.lock().unwrap().current_desktop_file.clone().filter(|p| p.exists());
        if cfg.set_desktop {
            let smooth = cfg.smooth_transitions && desktop_file == other_file;
            apply_desktop(app, desktop_file.clone(), other_file.clone(), prev_desktop.clone(), smooth).await?;
            self.state.lock().unwrap().current_desktop_file = Some(desktop_file.clone());
        }
        if let Some(lock) = lock_file.clone() {
            // Don't stack the lock-screen update on top of the desktop
            // change: each makes Windows decode an image, and together they
            // make a longer hitch.
            if cfg.set_desktop {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let allow_policy = cfg.allow_policy_lock_screen;
            let res = tauri::async_runtime::spawn_blocking(move || wallsetter::set_lock_screen(&lock, allow_policy))
                .await
                .map_err(|e| e.to_string())?;
            self.status.lock().unwrap().lock_screen_note = match res {
                Ok(wallsetter::LockScreenMethod::Policy) => {
                    Some("Lock screen set via system policy (admin).".into())
                }
                Ok(_) => None,
                Err(e) => Some(format!("Lock screen not changed: {e}")),
            };
        }

        // Keep the previous picture too: Windows may still be fading from it.
        cleanup(dir, &[Some(desktop_orig), lock_orig, Some(desktop_file), Some(other_file), lock_file, prev_desktop]);
        let state = self.state.lock().unwrap().clone();
        let _ = config::save_state(app, &state);
        Ok(())
    }

    /// Background loop: optional change at startup, then every
    /// `interval_minutes` measured from the last change (kept across restarts).
    pub async fn run_scheduler(&self, app: AppHandle) {
        // Was the app just started because the computer started / user signed in?
        let session = wallsetter::system::session_start_time();
        let fresh_start = {
            let mut st = self.state.lock().unwrap();
            let fresh = schedule::is_new_session(st.last_session_start, session);
            st.last_session_start = session;
            fresh
        };
        let state = self.state.lock().unwrap().clone();
        let _ = config::save_state(&app, &state);

        let mut force = fresh_start && self.config().change_on_startup;
        // A change the user just asked for (settings saved) skips the game check.
        let mut user_asked = false;
        // Fill the cache even if nothing changes right now.
        self.settings_changed(&app);

        loop {
            let cfg = self.config();
            let ready = !cfg.paused && !cfg.share_url.trim().is_empty();
            let last = self.state.lock().unwrap().last_changed;
            if ready && (force || schedule::secs_until_due(last, cfg.interval_minutes, now_secs()) == 0) {
                if !user_asked {
                    self.wait_out_fullscreen().await;
                }
                let _ = self.rotate(&app).await;
            }
            force = false;
            user_asked = false;
            let last = self.state.lock().unwrap().last_changed;
            // Retry after a minute if a change failed and none has succeeded yet.
            let wait = schedule::secs_until_due(last, cfg.interval_minutes, now_secs()).max(60);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(wait)), if ready => {}
                // Settings saved / resumed: change now so the user sees the effect.
                _ = self.wake.notified() => { force = true; user_asked = true; }
            }
        }
    }

    /// Hold off while a full-screen game/app is running (if enabled), so the
    /// wallpaper change can't cause a stutter mid-game.
    async fn wait_out_fullscreen(&self) {
        loop {
            if !self.config().pause_during_fullscreen {
                return;
            }
            let busy = tauri::async_runtime::spawn_blocking(wallsetter::system::fullscreen_app_running)
                .await
                .unwrap_or(false);
            if !busy {
                return;
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    }

    /// Watch for the screen being unlocked and change the wallpaper then.
    pub async fn run_unlock_watcher(&self, app: AppHandle) {
        const MIN_GAP_SECS: u64 = 60;
        let mut was_locked = false;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let cfg = self.config();
            if !cfg.change_on_unlock || cfg.paused || cfg.share_url.trim().is_empty() {
                was_locked = false;
                continue;
            }
            let locked = tauri::async_runtime::spawn_blocking(wallsetter::system::is_screen_locked)
                .await
                .ok()
                .flatten();
            match locked {
                Some(true) => was_locked = true,
                Some(false) if was_locked => {
                    was_locked = false;
                    let last = self.state.lock().unwrap().last_changed.unwrap_or(0);
                    if now_secs().saturating_sub(last) >= MIN_GAP_SECS {
                        self.wait_out_fullscreen().await;
                        let _ = self.rotate(&app).await;
                    }
                }
                _ => {}
            }
        }
    }
}

async fn download_to(
    client: &ImmichClient,
    asset: &Asset,
    prefer_original: bool,
    dir: &Path,
    prefix: &str,
    stamp: u64,
) -> Result<PathBuf, String> {
    let img = client.download(asset, prefer_original).await.map_err(|e| e.to_string())?;
    // Unique name per change: Windows and macOS cache wallpapers by path.
    let path = dir.join(format!("{prefix}-{stamp}.{}", img.ext));
    std::fs::write(&path, &img.bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Delete wallpapers other than the ones in use.
fn cleanup(dir: &Path, keep: &[Option<PathBuf>]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if !keep.iter().flatten().any(|k| *k == p) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Set `main` on the main monitor and `others` on the rest.
#[cfg(target_os = "macos")]
async fn apply_desktop(
    app: &AppHandle,
    main: PathBuf,
    others: PathBuf,
    _prev: Option<PathBuf>,
    _smooth: bool,
) -> Result<(), String> {
    // AppKit needs the main thread.
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(wallsetter::set_desktop_split(&main, &others));
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

/// With `smooth` (Windows), go through Windows' slideshow so it crossfades
/// from `prev` instead of swapping abruptly.
#[cfg(not(target_os = "macos"))]
async fn apply_desktop(
    _app: &AppHandle,
    main: PathBuf,
    others: PathBuf,
    prev: Option<PathBuf>,
    smooth: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if main == others && smooth {
            wallsetter::set_desktop_slideshow(prev.as_deref(), &main)
        } else if main == others {
            wallsetter::set_desktop(&main)
        } else {
            wallsetter::set_desktop_split(&main, &others)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Number of monitors (AppKit needs the main thread on macOS).
pub async fn monitor_count(app: &AppHandle) -> usize {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if app.run_on_main_thread(move || { let _ = tx.send(wallsetter::monitor_count()); }).is_err() {
            return 1;
        }
        rx.await.unwrap_or(1)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        wallsetter::monitor_count()
    }
}

/// Largest width and height across all monitors, in physical pixels.
/// Photos are shrunk to cover this so the OS has less to decode.
fn screen_fit(app: &AppHandle) -> Option<(u32, u32)> {
    let monitors = app.available_monitors().ok()?;
    let w = monitors.iter().map(|m| m.size().width).max()?;
    let h = monitors.iter().map(|m| m.size().height).max()?;
    (w > 0 && h > 0).then_some((w, h))
}

/// Apply dim/blur to `orig` into `<prefix>-<stamp>.jpg` off the async runtime (CPU work).
async fn processed(
    orig: &Path,
    dir: &Path,
    prefix: &str,
    stamp: u64,
    fx: crate::effects::Effects,
    fit: Option<(u32, u32)>,
) -> Result<PathBuf, String> {
    let (orig, dst) = (orig.to_path_buf(), dir.join(format!("{prefix}-{stamp}.jpg")));
    // Low priority: this work must never take CPU from games or the desktop.
    tauri::async_runtime::spawn_blocking(move || {
        wallsetter::system::in_background(|| crate::effects::apply_effects(&orig, &dst, fx, fit))
    })
        .await
        .map_err(|e| e.to_string())?
}

/// Used in file names: unique per change even when changes come quickly.
fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Convenience accessor.
pub fn engine(app: &AppHandle) -> tauri::State<'_, Engine> {
    app.state::<Engine>()
}
