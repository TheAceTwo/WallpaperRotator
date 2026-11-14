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

// ---------- session start (for "change when the computer starts") ----------

/// Unix seconds when the current sign-in session began.
///
/// Windows: the session's logon time. This changes on every real sign-in,
/// including after a "shut down" with Fast Startup (which does not reset the
/// boot clock). macOS: the system boot time.
#[cfg(windows)]
pub fn session_start_time() -> Option<u64> {
    let info = windows_session::query()?;
    // FILETIME (100 ns since 1601) -> Unix seconds.
    let secs = (info.logon_time as u64) / 10_000_000;
    secs.checked_sub(11_644_473_600)
}

#[cfg(target_os = "macos")]
pub fn session_start_time() -> Option<u64> {
    let mut tv = libc::timeval { tv_sec: 0, tv_usec: 0 };
    let mut size = std::mem::size_of::<libc::timeval>();
    let name = c"kern.boottime";
    // SAFETY: valid NUL-terminated name and correctly sized output buffer.
    let rc = unsafe {
        libc::sysctlbyname(name.as_ptr(), (&mut tv as *mut libc::timeval).cast(), &mut size, std::ptr::null_mut(), 0)
    };
    (rc == 0 && tv.tv_sec > 0).then_some(tv.tv_sec as u64)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn session_start_time() -> Option<u64> {
    None
}

// ---------- screen lock state (for "change when I unlock") ----------

/// `Some(true)` while the screen is locked; `None` if it can't be determined.
#[cfg(windows)]
pub fn is_screen_locked() -> Option<bool> {
    use windows::Win32::System::RemoteDesktop::WTS_SESSIONSTATE_LOCK;
    windows_session::query().map(|i| i.flags == WTS_SESSIONSTATE_LOCK as i32)
}

#[cfg(target_os = "macos")]
pub fn is_screen_locked() -> Option<bool> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> *mut c_void;
    }
    // SAFETY: "Copy" rule: we own the returned CFDictionary, which is
    // toll-free bridged to NSDictionary.
    let dict: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> =
        unsafe { Retained::from_raw(CGSessionCopyCurrentDictionary().cast())? };
    let locked = dict
        .objectForKey(&NSString::from_str("CGSSessionScreenIsLocked"))
        .and_then(|v| v.downcast::<NSNumber>().ok())
        .is_some_and(|n| n.boolValue());
    Some(locked)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn is_screen_locked() -> Option<bool> {
    None
}

#[cfg(windows)]
mod windows_session {
    use windows::core::PWSTR;
    use windows::Win32::System::RemoteDesktop::{
        WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW, WTS_CURRENT_SERVER_HANDLE,
        WTS_CURRENT_SESSION,
    };

    pub struct Info {
        pub flags: i32,
        pub logon_time: i64,
    }

    pub fn query() -> Option<Info> {
        let mut buf = PWSTR::null();
        let mut len = 0u32;
        // SAFETY: Windows allocates `buf`; we read it as WTSINFOEXW (as
        // documented for WTSSessionInfoEx) and free it with WTSFreeMemory.
        unsafe {
            WTSQuerySessionInformationW(
                Some(WTS_CURRENT_SERVER_HANDLE),
                WTS_CURRENT_SESSION,
                WTSSessionInfoEx,
                &mut buf,
                &mut len,
            )
            .ok()?;
            let result = if (len as usize) >= std::mem::size_of::<WTSINFOEXW>() {
                let info = &*(buf.0 as *const WTSINFOEXW);
                (info.Level == 1).then(|| {
                    let l1 = info.Data.WTSInfoExLevel1;
                    Info { flags: l1.SessionFlags, logon_time: l1.LogonTime }
                })
            } else {
                None
            };
            WTSFreeMemory(buf.0.cast());
            result
        }
    }
}

// ---------- start-at-sign-in entry left by an older app name ----------

/// Remove a sign-in entry registered under `old_name` (from before the app
/// was renamed). Returns true if one existed.
#[cfg(any(windows, target_os = "macos"))]
pub fn remove_legacy_autostart(old_name: &str) -> bool {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let Ok(al) = auto_launch::AutoLaunchBuilder::new()
        .set_app_name(old_name)
        .set_app_path(&exe)
        .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
        .build()
    else {
        return false;
    };
    if al.is_enabled().unwrap_or(false) {
        let _ = al.disable();
        true
    } else {
        false
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn remove_legacy_autostart(_old_name: &str) -> bool {
    false
}
