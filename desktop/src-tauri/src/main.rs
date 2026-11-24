// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod diag;
mod effects;
mod engine;
mod flyout;
mod schedule;
mod updater;

/// Shown in the tray, window and OS. Keep in sync with tauri.conf.json productName.
const APP_NAME: &str = "Cody’s Wallpapers";
/// Name used before the rename; its start-at-sign-in entry is cleaned up.
const LEGACY_APP_NAME: &str = "Wallpaper Rotator";

use config::Config;
use engine::{engine, Engine, Status};
use immich_core::{ImmichClient, ShareLink};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

const TRAY_ID: &str = "main";

// ---------- commands used by the settings page ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    os: &'static str,
    separate_lock_screen: bool,
}

#[tauri::command]
fn platform() -> Platform {
    Platform {
        os: std::env::consts::OS,
        separate_lock_screen: wallsetter::SUPPORTS_SEPARATE_LOCK_SCREEN,
    }
}

#[tauri::command]
fn get_config(app: AppHandle) -> Config {
    engine(&app).config()
}

#[tauri::command]
fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    let mut config = config;
    config.migrate();
    for a in &mut config.albums {
        a.url = a.url.trim().to_string();
        if !a.url.is_empty() {
            ShareLink::parse(&a.url).map_err(|e| format!("{}: {e}", if a.name.is_empty() { &a.url } else { &a.name }))?;
        }
    }
    config.albums.retain(|a| !a.url.is_empty());
    if config.use_backup_domain && !config.backup_domain.trim().is_empty() {
        config.backup_domain =
            immich_core::parse_server(&config.backup_domain).map_err(|e| format!("Backup address: {e}"))?;
    }
    config.interval_minutes = config.interval_minutes.clamp(1, 7 * 24 * 60);
    config::save_config(&app, &config).map_err(|e| e.to_string())?;
    let eng = engine(&app);
    let old = eng.config();
    let links_changed = old.albums != config.albums
        || old.backup_domain != config.backup_domain
        || old.use_backup_domain != config.use_backup_domain;
    *eng.config.lock().unwrap() = config;
    if links_changed {
        eng.invalidate_cache();
    }
    // Drop cached photos that no longer fit (other albums/orientation) and refill.
    eng.settings_changed(&app);
    refresh_tray(&app);
    eng.wake.notify_one();
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumInfo {
    name: Option<String>,
    photo_count: usize,
}

/// Check an album link (on its own address, then the backup address if
/// given): its name and how many photos it has.
#[tauri::command]
async fn inspect_album(url: String, backup_domain: Option<String>) -> Result<AlbumInfo, String> {
    let link = ShareLink::parse(&url).map_err(|e| e.to_string())?;
    let mut servers = vec![link.server.clone()];
    if let Some(b) = backup_domain.filter(|b| !b.trim().is_empty()) {
        servers.push(immich_core::parse_server(&b).map_err(|e| format!("Backup address: {e}"))?);
    }
    let mut errors = Vec::new();
    for server in servers {
        let client = ImmichClient::new(link.with_server(&server)).map_err(|e| e.to_string())?;
        match client.list_assets().await {
            Ok(assets) => {
                let name = client.album_name().await.ok().flatten();
                return Ok(AlbumInfo { name, photo_count: assets.iter().filter(|a| a.is_image).count() });
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    Err(errors.join(" · "))
}

/// Check that an album is reachable through the backup address.
#[tauri::command]
async fn test_backup_domain(domain: String, album_url: String) -> Result<usize, String> {
    let server = immich_core::parse_server(&domain).map_err(|e| e.to_string())?;
    let link = ShareLink::parse(&album_url).map_err(|e| e.to_string())?.with_server(&server);
    let client = ImmichClient::new(link).map_err(|e| e.to_string())?;
    let assets = client.list_assets().await.map_err(|e| e.to_string())?;
    Ok(assets.iter().filter(|a| a.is_image).count())
}

#[tauri::command]
async fn next_wallpaper(app: AppHandle) -> Result<Status, String> {
    let eng = engine(&app);
    eng.rotate(&app).await?;
    Ok(eng.status())
}

#[tauri::command]
fn get_status(app: AppHandle) -> Status {
    engine(&app).status()
}

/// Dim/blur settings sent by the settings page.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectSettings {
    dim_percent: u8,
    blur_level: u8,
    dim_lock_screen: bool,
    separate_other_monitors: bool,
    other_dim_percent: u8,
    other_blur_level: u8,
}

/// Save dim/blur settings and re-apply the current wallpaper with them.
#[tauri::command]
async fn set_effects(app: AppHandle, effects: EffectSettings) -> Result<(), String> {
    let eng = engine(&app);
    let mut cfg = eng.config();
    cfg.dim_percent = effects.dim_percent.min(80);
    cfg.blur_level = effects.blur_level.min(10);
    cfg.dim_lock_screen = effects.dim_lock_screen;
    cfg.separate_other_monitors = effects.separate_other_monitors;
    cfg.other_dim_percent = effects.other_dim_percent.min(80);
    cfg.other_blur_level = effects.other_blur_level.min(10);
    config::save_config(&app, &cfg).map_err(|e| e.to_string())?;
    *eng.config.lock().unwrap() = cfg;
    eng.reapply(&app).await
}

/// How many monitors are connected (decides whether per-monitor options show).
#[tauri::command]
async fn monitor_count(app: AppHandle) -> usize {
    engine::monitor_count(&app).await
}

/// System accent color as "#rrggbb", or null if the OS has none.
#[tauri::command]
fn accent_color() -> Option<String> {
    wallsetter::system::accent_color().map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"))
}

/// page: "background" or "lockscreen"
#[tauri::command]
fn open_wallpaper_settings(page: String) -> Result<(), String> {
    use wallsetter::system::SettingsPage;
    let page = if page == "lockscreen" { SettingsPage::LockScreen } else { SettingsPage::Background };
    wallsetter::system::open_wallpaper_settings(page)
}

/// Send the settings window back to the tray (used after 20 idle minutes).
#[tauri::command]
fn hide_settings(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.hide();
    }
}

/// Windows: number of virtual desktops if per-desktop wallpapers work on
/// this PC (Windows 11 24H2+), else null. macOS: always available (Spaces).
#[tauri::command]
async fn virtual_desktops() -> Option<usize> {
    if cfg!(target_os = "macos") {
        return Some(0);
    }
    tauri::async_runtime::spawn_blocking(wallsetter::system::virtual_desktop_count).await.ok().flatten()
}

/// Open the diagnostics log (one line per wallpaper change) in a text editor.
#[tauri::command]
fn open_log(app: AppHandle) -> Result<(), String> {
    diag::open(&app)
}

/// Version of an update that was found but not installed yet.
#[tauri::command]
fn pending_update(app: AppHandle) -> Option<String> {
    updater::pending_version(&app)
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Result<Option<updater::UpdateInfo>, String> {
    updater::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install(&app).await
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

// ---------- tray ----------

// ---------- tray flyout ----------

/// Flyout size in logical pixels (scaled per monitor).
const FLYOUT_W: f64 = 400.0;
const FLYOUT_H: f64 = 680.0;

#[derive(Default)]
struct FlyoutState {
    /// Last known tray icon rectangle (physical pixels).
    icon: Mutex<Option<flyout::Area>>,
    /// When the flyout last hid itself on losing focus.
    hidden_at: Mutex<Option<Instant>>,
}

/// Size and place the flyout next to the tray icon, inside the work area.
fn position_flyout(app: &AppHandle, w: &WebviewWindow) {
    let icon = *app.state::<FlyoutState>().icon.lock().unwrap();
    let monitor = icon
        .and_then(|i| {
            app.monitor_from_point(f64::from(i.x) + f64::from(i.w) / 2.0, f64::from(i.y) + f64::from(i.h) / 2.0)
                .ok()
                .flatten()
        })
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let wa = m.work_area();
    let work = flyout::Area { x: wa.position.x, y: wa.position.y, w: wa.size.width, h: wa.size.height };
    let scale = m.scale_factor();
    let (win_w, win_h) = ((FLYOUT_W * scale) as u32, (FLYOUT_H * scale) as u32);
    let (x, y, h) = flyout::place(icon, work, win_w, win_h);
    let _ = w.set_size(PhysicalSize::new(win_w, h));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        position_flyout(app, &w);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Tray icon left-click: open the flyout, or close it if it's open.
fn toggle_flyout(app: &AppHandle) {
    let Some(w) = app.get_webview_window("settings") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    // Clicking the icon first takes focus from the open flyout, which hides
    // it; don't immediately reopen it in that case.
    let just_hid = app
        .state::<FlyoutState>()
        .hidden_at
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400));
    if !just_hid {
        show_settings(app);
    }
}

/// Hide the "drag the icon out of ^" tip for good.
#[tauri::command]
fn dismiss_tray_hint(app: AppHandle) -> Result<(), String> {
    let eng = engine(&app);
    let mut cfg = eng.config();
    cfg.tray_hint_dismissed = true;
    config::save_config(&app, &cfg).map_err(|e| e.to_string())?;
    *eng.config.lock().unwrap() = cfg;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let paused = engine(app).config().paused;
    let update_item = match updater::pending_version(app) {
        Some(v) => MenuItem::with_id(app, "install-update", format!("Update to v{v}"), true, None::<&str>)?,
        None => MenuItem::with_id(app, "check-update", "Check for updates", true, None::<&str>)?,
    };
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "next", "Next wallpaper", true, None::<&str>)?,
            &CheckMenuItem::with_id(app, "pause", "Pause", true, paused, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
            &update_item,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )
}

pub(crate) fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn spawn_rotate(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = engine(&app).rotate(&app).await;
    });
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(APP_NAME)
        .menu(&build_menu(app)?)
        // Left-click opens the flyout; right-click (Control-click on Mac) the menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "next" => spawn_rotate(app),
            "pause" => {
                let eng = engine(app);
                let mut cfg = eng.config();
                cfg.paused = !cfg.paused;
                let _ = config::save_config(app, &cfg);
                *eng.config.lock().unwrap() = cfg;
                eng.wake.notify_one();
                refresh_tray(app);
            }
            "settings" => show_settings(app),
            "check-update" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(None) = updater::check(&app).await {
                        let _ = app.emit("update-none", ());
                    }
                });
            }
            "install-update" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = updater::install(&app).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { rect, button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                event
            {
                let app = tray.app_handle();
                let pos = rect.position.to_physical::<i32>(1.0);
                let size = rect.size.to_physical::<u32>(1.0);
                *app.state::<FlyoutState>().icon.lock().unwrap() =
                    Some(flyout::Area { x: pos.x, y: pos.y, w: size.width, h: size.height });
                toggle_flyout(app);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        // A second launch just opens the existing instance's settings.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .invoke_handler(tauri::generate_handler![
            platform,
            get_config,
            save_config,
            inspect_album,
            test_backup_domain,
            next_wallpaper,
            get_status,
            get_autostart,
            set_autostart,
            app_version,
            check_update,
            install_update,
            accent_color,
            open_wallpaper_settings,
            set_effects,
            monitor_count,
            pending_update,
            hide_settings,
            virtual_desktops,
            open_log,
            dismiss_tray_hint
        ])
        .setup(|app| {
            // Menu-bar only on macOS (no Dock icon).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            app.manage(Engine::new(&handle));
            app.manage(updater::Pending::default());
            app.manage(FlyoutState::default());
            setup_tray(&handle)?;

            // First run (no link yet) or a manual launch: show settings.
            let launched_at_login = std::env::args().any(|a| a == "--background");
            if !engine(&handle).config().has_albums() || !launched_at_login {
                show_settings(&handle);
            }

            // Renamed from "Wallpaper Rotator": move its start-at-sign-in entry
            // to the new name so the app neither launches twice nor disappears.
            if wallsetter::system::remove_legacy_autostart(LEGACY_APP_NAME) {
                let _ = handle.autolaunch().enable();
            }

            let sched = handle.clone();
            tauri::async_runtime::spawn(async move {
                engine(&sched).run_scheduler(sched.clone()).await;
            });
            let spaces = handle.clone();
            tauri::async_runtime::spawn(async move {
                engine(&spaces).run_space_watcher(spaces.clone()).await;
            });
            let unlock = handle.clone();
            tauri::async_runtime::spawn(async move {
                engine(&unlock).run_unlock_watcher(unlock.clone()).await;
            });
            tauri::async_runtime::spawn(updater::run_update_loop(handle.clone()));
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the settings window keeps the app running in the tray.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Like a system flyout: clicking elsewhere tucks it away. Not during
            // first-time setup (the share link is often copied from a browser)
            // or while an update installs (so its progress stays visible).
            WindowEvent::Focused(false) => {
                let app = window.app_handle();
                let configured = engine(app).config().has_albums();
                let installing = updater::INSTALLING.load(std::sync::atomic::Ordering::SeqCst);
                if configured && !installing && window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                    *app.state::<FlyoutState>().hidden_at.lock().unwrap() = Some(Instant::now());
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("failed to start Cody’s Wallpapers")
        .run(|_app, event| {
            // Keep running with no windows open.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
