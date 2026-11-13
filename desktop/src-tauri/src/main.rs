// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;

use config::Config;
use engine::{engine, Engine, Status};
use immich_core::{ImmichClient, ShareLink};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent, Wry};
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
    if !config.share_url.trim().is_empty() {
        ShareLink::parse(&config.share_url).map_err(|e| e.to_string())?;
    }
    let mut config = config;
    config.interval_minutes = config.interval_minutes.clamp(1, 7 * 24 * 60);
    config::save_config(&app, &config).map_err(|e| e.to_string())?;
    let eng = engine(&app);
    let link_changed = eng.config().share_url != config.share_url;
    *eng.config.lock().unwrap() = config;
    if link_changed {
        eng.invalidate_cache();
    }
    refresh_tray(&app);
    eng.wake.notify_one();
    Ok(())
}

/// Check a share link without saving it. Returns the number of photos.
#[tauri::command]
async fn test_link(share_url: String) -> Result<usize, String> {
    let link = ShareLink::parse(&share_url).map_err(|e| e.to_string())?;
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

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let paused = engine(app).config().paused;
    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "next", "Next wallpaper", true, None::<&str>)?,
            &CheckMenuItem::with_id(app, "pause", "Pause", true, paused, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )
}

fn refresh_tray(app: &AppHandle) {
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
        .tooltip("Wallpaper Rotator")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
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
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Windows: left-click the tray icon to open settings.
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                if !cfg!(target_os = "macos") {
                    show_settings(tray.app_handle());
                }
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
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--background"])))
        .invoke_handler(tauri::generate_handler![
            platform,
            get_config,
            save_config,
            test_link,
            next_wallpaper,
            get_status,
            get_autostart,
            set_autostart
        ])
        .setup(|app| {
            // Menu-bar only on macOS (no Dock icon).
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            app.manage(Engine::new(&handle));
            setup_tray(&handle)?;

            // First run (no link yet) or a manual launch: show settings.
            let launched_at_login = std::env::args().any(|a| a == "--background");
            if engine(&handle).config().share_url.is_empty() || !launched_at_login {
                show_settings(&handle);
            }

            let sched = handle.clone();
            tauri::async_runtime::spawn(async move {
                engine(&sched).run_scheduler(sched.clone()).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window keeps the app running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to start Wallpaper Rotator")
        .run(|_app, event| {
            // Keep running with no windows open.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
