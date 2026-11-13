use crate::Result;
use std::path::Path;
use windows::core::{HSTRING, PCWSTR};
use windows::Storage::StorageFile;
use windows::System::UserProfile::LockScreen;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_FILL};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockScreenMethod {
    /// Per-user WinRT API (no admin; works on Home/Pro).
    WinRt,
    /// Machine-wide PersonalizationCSP policy (needs admin; locks the setting).
    Policy,
}

/// Set the desktop wallpaper on every monitor, scaled to fill.
///
/// Call from a background thread (it initialises COM on that thread).
pub fn set_desktop(path: &Path) -> Result<()> {
    let wide = HSTRING::from(path.as_os_str());
    match set_desktop_com(&wide) {
        Ok(()) => Ok(()),
        Err(com_err) => set_desktop_spi(&wide)
            .map_err(|spi_err| format!("IDesktopWallpaper: {com_err}; SystemParametersInfo: {spi_err}")),
    }
}

fn set_desktop_com(path: &HSTRING) -> windows::core::Result<()> {
    unsafe {
        // S_FALSE (already initialised) is fine; only balance a successful init.
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)?;
            dw.SetPosition(DWPOS_FILL)?;
            // A null monitor id applies the image to all monitors.
            dw.SetWallpaper(PCWSTR::null(), path)
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

fn set_desktop_spi(path: &HSTRING) -> windows::core::Result<()> {
    let mut buf: Vec<u16> = path.iter().copied().chain(std::iter::once(0)).collect();
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(buf.as_mut_ptr().cast()),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
}

/// Set the lock screen image.
///
/// Uses the per-user WinRT API first. If that fails and `allow_policy` is
/// set, writes the PersonalizationCSP policy keys (requires running as
/// administrator; once set, Windows Settings shows the lock screen as
/// "managed by your organization").
///
/// Note: if the user has "Windows spotlight" selected for the lock screen,
/// Windows may keep showing Spotlight; set Settings > Personalization >
/// Lock screen to "Picture" once.
pub fn set_lock_screen(path: &Path, allow_policy: bool) -> Result<LockScreenMethod> {
    let winrt = (|| -> windows::core::Result<()> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.get()?;
        LockScreen::SetImageFileAsync(&file)?.get()
    })();
    match winrt {
        Ok(()) => Ok(LockScreenMethod::WinRt),
        Err(e) if allow_policy => set_lock_policy(path)
            .map(|_| LockScreenMethod::Policy)
            .map_err(|p| format!("WinRT: {e}; policy fallback: {p}")),
        Err(e) => Err(format!("WinRT lock screen API failed: {e}")),
    }
}

fn set_lock_policy(path: &Path) -> std::io::Result<()> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_WRITE};
    use winreg::RegKey;
    let (key, _) = RegKey::predef(HKEY_LOCAL_MACHINE).create_subkey_with_flags(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\PersonalizationCSP",
        KEY_WRITE,
    )?;
    let p = path.to_string_lossy().to_string();
    key.set_value("LockScreenImagePath", &p)?;
    key.set_value("LockScreenImageUrl", &p)?;
    key.set_value("LockScreenImageStatus", &1u32)?;
    Ok(())
}
