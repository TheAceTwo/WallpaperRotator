//! Self-update from GitHub Releases via tauri-plugin-updater.
//!
//! The plugin fetches `latest.json` from the latest *published* release
//! (see `plugins.updater` in tauri.conf.json), verifies the download's
//! signature against the embedded public key, and installs it.

use crate::engine::engine;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// An update that was found but not installed yet (auto-update off, or the
/// install failed).
#[derive(Default)]
pub struct Pending(pub Mutex<Option<Update>>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

pub fn pending_version(app: &AppHandle) -> Option<String> {
    app.state::<Pending>().0.lock().unwrap().as_ref().map(|u| u.version.clone())
}

/// Ask GitHub for a newer version. Stores it as pending if found.
pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone() });
    *app.state::<Pending>().0.lock().unwrap() = update;
    crate::refresh_tray(app);
    if let Some(i) = &info {
        let _ = app.emit("update-available", i.clone());
    }
    Ok(info)
}

/// Download and install the pending update, then relaunch.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let update = app
        .state::<Pending>()
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("No update is waiting to be installed.")?;
    let _ = app.emit("update-installing", update.version.clone());
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| format!("Update failed: {e}"))?;
    // On Windows the installer has already closed the app at this point;
    // on macOS the bundle was replaced in place and we relaunch it.
    app.restart();
}

/// Background loop: check shortly after launch, then daily.
pub async fn run_update_loop(app: AppHandle) {
    if cfg!(debug_assertions) {
        return; // dev builds never self-update
    }
    tokio::time::sleep(FIRST_CHECK).await;
    loop {
        if let Ok(Some(_)) = check(&app).await {
            if engine(&app).config().auto_update {
                let _ = install(&app).await;
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}
