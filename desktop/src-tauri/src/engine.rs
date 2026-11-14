//! Fetch → pick → download → apply, plus the background schedule.

use crate::config::{self, Config, State};
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
    /// Serialises rotations so the timer and "Next" can't overlap.
    busy: tokio::sync::Mutex<()>,
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
            busy: tokio::sync::Mutex::new(()),
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

    async fn assets(&self, client: &ImmichClient, share_url: &str) -> Result<Vec<Asset>, String> {
        if let Some(c) = self.cache.lock().unwrap().as_ref() {
            if c.share_url == share_url && c.fetched.elapsed() < LIST_CACHE {
                return Ok(c.assets.clone());
            }
        }
        let assets = client.list_assets().await.map_err(|e| e.to_string())?;
        *self.cache.lock().unwrap() = Some(ListCache {
            share_url: share_url.to_string(),
            fetched: Instant::now(),
            assets: assets.clone(),
        });
        Ok(assets)
    }

    /// Change the wallpaper now. Records the outcome in `status`.
    pub async fn rotate(&self, app: &AppHandle) -> Result<(), String> {
        let _guard = self.busy.lock().await;
        let result = self.rotate_inner(app).await;
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
        let _ = app.emit("status", self.status());
        result
    }

    async fn rotate_inner(&self, app: &AppHandle) -> Result<(), String> {
        let cfg = self.config();
        if cfg.share_url.trim().is_empty() {
            return Err("Add your Immich share link in Settings first.".into());
        }
        let client = ImmichClient::new(ShareLink::parse(&cfg.share_url).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let assets = self.assets(&client, &cfg.share_url).await?;
        self.status.lock().unwrap().photo_count = Some(assets.iter().filter(|a| a.is_image).count());

        let want_lock = cfg.set_lock_screen && wallsetter::SUPPORTS_SEPARATE_LOCK_SCREEN;
        let (desktop_pick, lock_pick) = {
            let mut st = self.state.lock().unwrap();
            let mut rng = rand::thread_rng();
            let d = immich_core::pick(&assets, cfg.orientation, &mut st.history, &mut rng).cloned();
            let l = if want_lock && cfg.separate_lock_image {
                immich_core::pick(&assets, cfg.orientation, &mut st.history, &mut rng).cloned()
            } else {
                d.clone()
            };
            (d, l)
        };
        let desktop_pick = desktop_pick.ok_or("The shared album has no photos.")?;

        let dir = config::wallpaper_dir(app);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = now_millis();

        let desktop_orig =
            download_to(&client, &desktop_pick, cfg.prefer_original, &dir, "desktop-orig", stamp).await?;
        let lock_orig = match &lock_pick {
            Some(l) if want_lock && l.id != desktop_pick.id => {
                Some(download_to(&client, l, cfg.prefer_original, &dir, "lock-orig", stamp).await?)
            }
            Some(_) if want_lock => Some(desktop_orig.clone()),
            _ => None,
        };
        {
            let mut st = self.state.lock().unwrap();
            st.current_desktop_original = Some(desktop_orig.clone());
            st.current_lock_original = lock_orig.clone();
        }
        self.apply_originals(app, &cfg, &dir, desktop_orig, lock_orig, stamp).await
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
        let desktop_dim = cfg.dim_percent;
        let lock_dim = if cfg.dim_lock_screen { cfg.dim_percent } else { 0 };

        let desktop_file = dimmed(&desktop_orig, dir, "desktop", stamp, desktop_dim).await?;
        let lock_file = match lock_orig.clone() {
            // Same photo and same dimming as the desktop: reuse the file.
            Some(l) if l == desktop_orig && lock_dim == desktop_dim => Some(desktop_file.clone()),
            Some(l) => Some(dimmed(&l, dir, "lock", stamp, lock_dim).await?),
            None => None,
        };

        if cfg.set_desktop {
            apply_desktop(app, desktop_file.clone()).await?;
        }
        if let Some(lock) = lock_file.clone() {
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

        cleanup(dir, &[Some(desktop_orig), lock_orig, Some(desktop_file), lock_file]);
        let state = self.state.lock().unwrap().clone();
        let _ = config::save_state(app, &state);
        Ok(())
    }

    /// Background loop: rotate on start, then every `interval_minutes`.
    pub async fn run_scheduler(&self, app: AppHandle) {
        loop {
            let cfg = self.config();
            let ready = !cfg.paused && !cfg.share_url.trim().is_empty();
            if ready {
                let _ = self.rotate(&app).await;
            }
            let wait = Duration::from_secs(u64::from(cfg.interval_minutes.max(1)) * 60);
            tokio::select! {
                _ = tokio::time::sleep(wait), if ready => {}
                _ = self.wake.notified() => {}
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

#[cfg(target_os = "macos")]
async fn apply_desktop(app: &AppHandle, path: PathBuf) -> Result<(), String> {
    // AppKit needs the main thread.
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(wallsetter::set_desktop(&path));
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

#[cfg(not(target_os = "macos"))]
async fn apply_desktop(_app: &AppHandle, path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || wallsetter::set_desktop(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// Dim `orig` into `<prefix>-<stamp>.jpg` off the async runtime (CPU work).
async fn dimmed(orig: &Path, dir: &Path, prefix: &str, stamp: u64, percent: u8) -> Result<PathBuf, String> {
    let (orig, dst) = (orig.to_path_buf(), dir.join(format!("{prefix}-{stamp}.jpg")));
    tauri::async_runtime::spawn_blocking(move || crate::dim::dim_file(&orig, &dst, percent))
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
