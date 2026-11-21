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
            ensure_fill(&dw)?;
            // A null monitor id applies the image to all monitors.
            dw.SetWallpaper(PCWSTR::null(), path)
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

/// Set the "Fill" layout only if it isn't already: every SetPosition makes
/// Windows redraw the whole desktop, which is a visible hitch.
unsafe fn ensure_fill(dw: &IDesktopWallpaper) -> windows::core::Result<()> {
    if dw.GetPosition().ok() != Some(DWPOS_FILL) {
        dw.SetPosition(DWPOS_FILL)?;
    }
    Ok(())
}

/// Change the wallpaper through Windows' own slideshow, so Windows crossfades
/// from the current picture (`prev`) to `next` instead of swapping abruptly.
///
/// Windows' slideshow shows the same picture on every monitor. Falls back to
/// [`set_desktop`] if anything about the slideshow fails.
pub fn set_desktop_slideshow(prev: Option<&Path>, next: &Path) -> Result<()> {
    match slideshow_com(prev, next) {
        Ok(()) => Ok(()),
        Err(_) => set_desktop(next),
    }
}

fn slideshow_com(prev: Option<&Path>, next: &Path) -> windows::core::Result<()> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        ILFree, SHCreateShellItemArrayFromIDLists, SHParseDisplayName, DESKTOP_SLIDESHOW_OPTIONS, DSD_FORWARD,
    };
    // Longest tick Windows accepts; the app advances the slideshow itself.
    const NEVER_MS: u32 = u32::MAX;

    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> windows::core::Result<()> {
            let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)?;
            ensure_fill(&dw)?;

            // Slideshow list: [previous, next] so "forward" fades prev -> next.
            let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
            for p in prev.into_iter().chain(std::iter::once(next)) {
                let mut pidl = std::ptr::null_mut();
                SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut pidl, 0, None)?;
                pidls.push(pidl);
            }
            let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
            let items = SHCreateShellItemArrayFromIDLists(&consts);
            for p in pidls {
                ILFree(Some(p));
            }
            let items = items?;

            dw.SetSlideshowOptions(DESKTOP_SLIDESHOW_OPTIONS(0), NEVER_MS)?; // in order, no shuffle
            dw.SetSlideshow(&items)?;

            // After SetSlideshow Windows normally still shows `prev` (first in
            // the list); one step forward crossfades to `next`. Decide once,
            // so a path Windows reports differently can't cause extra steps.
            let current = dw.GetWallpaper(PCWSTR::null()).ok().map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            });
            let is = |path: &Path, cur: &str| cur.eq_ignore_ascii_case(&path.to_string_lossy());
            let already_next = current.as_deref().is_some_and(|c| is(next, c));
            if prev.is_some() && !already_next {
                dw.AdvanceSlideshow(PCWSTR::null(), DSD_FORWARD)?;
            }
            Ok(())
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

/// Number of active monitors.
pub fn monitor_count() -> usize {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
    unsafe { GetSystemMetrics(SM_CMONITORS).max(1) as usize }
}

/// Set `primary` on the main display and `others` on every other monitor.
pub fn set_desktop_split(primary: &Path, others: &Path) -> Result<()> {
    use windows::Win32::System::Com::CoTaskMemFree;
    let primary = HSTRING::from(primary.as_os_str());
    let others = HSTRING::from(others.as_os_str());
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> windows::core::Result<()> {
            let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)?;
            ensure_fill(&dw)?;
            for i in 0..dw.GetMonitorDevicePathCount()? {
                let id = dw.GetMonitorDevicePathAt(i)?;
                let monitor = PCWSTR(id.0);
                // Detached monitors have no rectangle: skip them.
                if let Ok(rect) = dw.GetMonitorRECT(monitor) {
                    // The primary display is the one whose top-left is (0, 0).
                    let path = if rect.left == 0 && rect.top == 0 { &primary } else { &others };
                    let r = dw.SetWallpaper(monitor, path);
                    CoTaskMemFree(Some(id.0 as _));
                    r?;
                } else {
                    CoTaskMemFree(Some(id.0 as _));
                }
            }
            Ok(())
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        result.map_err(|e| format!("couldn't set per-monitor wallpaper: {e}"))
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
