//! User settings and persisted state (JSON files in the app's data dirs).
//! The share link lives only here, on each user's machine; it is never
//! compiled into the app.

use immich_core::{History, Orientation};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Immich public share link, e.g. https://photos.example.com/share/XXXX
    pub share_url: String,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            share_url: String::new(),
            interval_minutes: 60,
            set_desktop: true,
            set_lock_screen: true,
            separate_lock_image: false,
            orientation: Orientation::Landscape,
            prefer_original: true,
            allow_policy_lock_screen: false,
            paused: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub history: History,
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
    load(&config_path(app))
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
