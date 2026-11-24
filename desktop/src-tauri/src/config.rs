//! User settings and persisted state (JSON files in the app's data dirs).
//! The share link lives only here, on each user's machine; it is never
//! compiled into the app.

use crate::effects::Effects;
use immich_core::{History, Orientation};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// One Immich shared album the photos come from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Album {
    /// Shown in the list (filled in from Immich when the album is added).
    pub name: String,
    /// Public share link, e.g. https://photos.example.com/s/family
    pub url: String,
}

/// Shuffle photos from every album, or show just one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlbumMode {
    #[default]
    All,
    One,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// The albums photos come from (share links live only here, on each
    /// user's machine).
    pub albums: Vec<Album>,
    pub album_mode: AlbumMode,
    /// With `AlbumMode::One`: the chosen album's link.
    pub selected_album: String,
    /// Also try the same album links on `backup_domain` (e.g. the home
    /// network or Tailscale address of the same Immich server) when the
    /// main address can't be reached. Only the server part changes.
    pub use_backup_domain: bool,
    /// e.g. https://192.168.1.123:2283
    pub backup_domain: String,
    /// Before 0.1.9: a single album link and a whole backup link. Read once
    /// and moved into `albums` / `backup_domain` (see `migrate`).
    #[serde(skip_serializing)]
    pub share_url: String,
    #[serde(skip_serializing)]
    pub use_backup_link: bool,
    #[serde(skip_serializing)]
    pub backup_share_url: String,
    pub interval_minutes: u32,
    pub set_desktop: bool,
    /// Windows only (macOS lock screen follows the desktop picture).
    pub set_lock_screen: bool,
    /// Use a different random photo for the lock screen (Windows).
    pub separate_lock_image: bool,
    pub orientation: Orientation,
    /// Use full-resolution JPEG/PNG originals when the link allows downloads.
    pub prefer_original: bool,
    /// Windows: fall back to the machine-wide lock screen policy (needs admin).
    pub allow_policy_lock_screen: bool,
    pub paused: bool,
    /// Download and install new releases from GitHub automatically.
    pub auto_update: bool,
    /// Darken wallpapers by this many percent (0 = off).
    pub dim_percent: u8,
    /// Blur strength 0–10 (0 = off).
    pub blur_level: u8,
    /// Windows: apply the main dim/blur to the lock screen too.
    pub dim_lock_screen: bool,
    /// Use separate dim/blur on monitors other than the main one.
    pub separate_other_monitors: bool,
    pub other_dim_percent: u8,
    pub other_blur_level: u8,
    /// Change the wallpaper on the first launch after the computer starts.
    pub change_on_startup: bool,
    /// Change the wallpaper every time the screen is unlocked.
    pub change_on_unlock: bool,
    /// Windows: hold timed/unlock changes while a full-screen game or app runs.
    pub pause_during_fullscreen: bool,
    /// Windows: change pictures through Windows' slideshow, which crossfades.
    pub smooth_transitions: bool,
    /// A different photo on each virtual desktop (Windows 11) / Space (macOS).
    pub per_virtual_desktop: bool,
    /// The "drag my icon out of the ^ menu" tip was dismissed.
    pub tray_hint_dismissed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            albums: Vec::new(),
            album_mode: AlbumMode::All,
            selected_album: String::new(),
            use_backup_domain: false,
            backup_domain: String::new(),
            share_url: String::new(),
            use_backup_link: false,
            backup_share_url: String::new(),
            interval_minutes: 60,
            set_desktop: true,
            set_lock_screen: true,
            separate_lock_image: false,
            orientation: Orientation::Landscape,
            prefer_original: true,
            allow_policy_lock_screen: false,
            paused: false,
            auto_update: true,
            dim_percent: 0,
            blur_level: 0,
            dim_lock_screen: true,
            separate_other_monitors: false,
            other_dim_percent: 0,
            other_blur_level: 0,
            change_on_startup: true,
            change_on_unlock: false,
            pause_during_fullscreen: true,
            smooth_transitions: true,
            per_virtual_desktop: false,
            tray_hint_dismissed: false,
        }
    }
}

impl Config {
    /// Move settings from before multiple albums into the new fields.
    pub fn migrate(&mut self) {
        let old = std::mem::take(&mut self.share_url);
        if self.albums.is_empty() && !old.trim().is_empty() {
            self.albums.push(Album { name: String::new(), url: old.trim().to_string() });
        }
        let old_backup = std::mem::take(&mut self.backup_share_url);
        if self.backup_domain.is_empty() && !old_backup.trim().is_empty() {
            if let Ok(server) = immich_core::parse_server(&old_backup) {
                self.backup_domain = server;
                self.use_backup_domain = self.use_backup_link;
            }
        }
        self.use_backup_link = false;
    }

    /// Is at least one album set up?
    pub fn has_albums(&self) -> bool {
        self.albums.iter().any(|a| !a.url.trim().is_empty())
    }

    /// The album links photos are drawn from right now.
    pub fn active_album_urls(&self) -> Vec<String> {
        let all: Vec<String> =
            self.albums.iter().map(|a| a.url.trim().to_string()).filter(|u| !u.is_empty()).collect();
        match self.album_mode {
            AlbumMode::All => all,
            AlbumMode::One => {
                let chosen = all.iter().find(|u| **u == self.selected_album.trim()).or(all.first());
                chosen.cloned().into_iter().collect()
            }
        }
    }

    /// The backup server address, when turned on and filled in.
    pub fn backup_server(&self) -> Option<&str> {
        let d = self.backup_domain.trim();
        (self.use_backup_domain && !d.is_empty()).then_some(d)
    }

    /// Dim/blur for the main monitor.
    pub fn main_effects(&self) -> Effects {
        Effects { dim: self.dim_percent, blur: self.blur_level }
    }

    /// Dim/blur for other monitors (same as main unless set separately).
    pub fn other_effects(&self) -> Effects {
        if self.separate_other_monitors {
            Effects { dim: self.other_dim_percent, blur: self.other_blur_level }
        } else {
            self.main_effects()
        }
    }

    /// Dim/blur for the lock screen (Windows).
    pub fn lock_effects(&self) -> Effects {
        if self.dim_lock_screen {
            self.main_effects()
        } else {
            Effects::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_single_link_and_backup_link() {
        let mut c = Config {
            share_url: "https://immich.theacetwo.net/s/family".into(),
            use_backup_link: true,
            backup_share_url: "http://192.168.1.123:2283/s/family".into(),
            ..Config::default()
        };
        c.migrate();
        assert_eq!(c.albums, vec![Album { name: String::new(), url: "https://immich.theacetwo.net/s/family".into() }]);
        assert_eq!(c.backup_domain, "http://192.168.1.123:2283");
        assert!(c.use_backup_domain);
        assert!(c.share_url.is_empty());
    }

    #[test]
    fn active_albums_follow_the_mode() {
        let a = |u: &str| Album { name: String::new(), url: u.into() };
        let mut c = Config { albums: vec![a("https://x/s/a"), a("https://x/s/b")], ..Config::default() };
        assert_eq!(c.active_album_urls().len(), 2, "shuffle all");
        c.album_mode = AlbumMode::One;
        c.selected_album = "https://x/s/b".into();
        assert_eq!(c.active_album_urls(), vec!["https://x/s/b"]);
        c.selected_album = "https://gone/s/z".into();
        assert_eq!(c.active_album_urls(), vec!["https://x/s/a"], "falls back to the first");
        assert_eq!(c.backup_server(), None);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub history: History,
    /// Undimmed copies of the photos currently in use, so dimming can be
    /// changed without downloading again.
    pub current_desktop_original: Option<PathBuf>,
    pub current_lock_original: Option<PathBuf>,
    /// The processed file currently on the desktop (where a crossfade starts).
    pub current_desktop_file: Option<PathBuf>,
    /// Originals for virtual desktops / Spaces 2..N this round.
    pub current_extra_originals: Vec<PathBuf>,
    /// This round's processed desktop pictures: [main, extras...]. macOS hands
    /// these out as you switch to Spaces that still show an older picture.
    pub round_desktop_files: Vec<PathBuf>,
    pub next_space_photo: usize,
    /// Photos downloaded ahead of time (see schedule::QUEUE_LEN).
    pub queue: Vec<crate::schedule::QueuedPhoto>,
    /// Session start seen at the last launch (detects a fresh start).
    pub last_session_start: Option<u64>,
    /// Unix seconds of the last successful change (keeps the timer across restarts).
    pub last_changed: Option<u64>,
}

fn config_path(app: &AppHandle) -> PathBuf {
    app.path().app_config_dir().expect("no config dir").join("config.json")
}

fn state_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no data dir").join("state.json")
}

/// Folder for downloaded wallpapers. Kept in the data dir (not a temp dir)
/// because the OS reads the file again on every login.
pub fn wallpaper_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no data dir").join("wallpapers")
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)
}

pub fn load_config(app: &AppHandle) -> Config {
    let mut c: Config = load(&config_path(app));
    let needs_migration = !c.share_url.is_empty() || !c.backup_share_url.is_empty();
    c.migrate();
    if needs_migration {
        let _ = save_config(app, &c);
    }
    c
}

pub fn save_config(app: &AppHandle, c: &Config) -> std::io::Result<()> {
    save(&config_path(app), c)
}

pub fn load_state(app: &AppHandle) -> State {
    load(&state_path(app))
}

pub fn save_state(app: &AppHandle, s: &State) -> std::io::Result<()> {
    save(&state_path(app), s)
}
