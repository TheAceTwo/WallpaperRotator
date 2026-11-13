//! OS look-and-feel helpers: accent color and opening the OS wallpaper settings.

use crate::Result;

/// Which settings page to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Background,
    /// Windows only; other OSes open their single wallpaper page.
    LockScreen,
}

/// The user's system accent color as sRGB, if the OS has one.
#[cfg(windows)]
pub fn accent_color() -> Option<[u8; 3]> {
    use windows::UI::ViewManagement::{UIColorType, UISettings};
    let c = UISettings::new().ok()?.GetColorValue(UIColorType::Accent).ok()?;
    Some([c.R, c.G, c.B])
}

#[cfg(target_os = "macos")]
pub fn accent_color() -> Option<[u8; 3]> {
    use objc2_app_kit::{NSColor, NSColorSpace};
    let accent = NSColor::controlAccentColor();
    let srgb = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([to8(srgb.redComponent()), to8(srgb.greenComponent()), to8(srgb.blueComponent())])
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn accent_color() -> Option<[u8; 3]> {
    None
}

/// Open the OS's own wallpaper settings.
#[cfg(windows)]
pub fn open_wallpaper_settings(page: SettingsPage) -> Result<()> {
    let uri = match page {
        SettingsPage::Background => "ms-settings:personalization-background",
        SettingsPage::LockScreen => "ms-settings:lockscreen",
    };
    // explorer.exe is a GUI program, so no console window flashes.
    std::process::Command::new("explorer.exe")
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open Settings: {e}"))
}

#[cfg(target_os = "macos")]
pub fn open_wallpaper_settings(_page: SettingsPage) -> Result<()> {
    use std::process::Command;
    // macOS 13+ (System Settings), then the older System Preferences pane.
    let modern = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Wallpaper-Settings.extension")
        .status();
    if matches!(modern, Ok(s) if s.success()) {
        return Ok(());
    }
    Command::new("open")
        .arg("/System/Library/PreferencePanes/DesktopScreenEffectsPref.prefPane")
        .status()
        .map_err(|e| format!("couldn't open System Settings: {e}"))
        .and_then(|s| if s.success() { Ok(()) } else { Err("couldn't open System Settings".into()) })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn open_wallpaper_settings(_page: SettingsPage) -> Result<()> {
    Err("opening wallpaper settings isn't supported on this OS".into())
}
