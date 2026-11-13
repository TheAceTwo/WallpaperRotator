//! Platform wallpaper setters.
//!
//! | OS      | Desktop                          | Lock screen                                   |
//! |---------|----------------------------------|-----------------------------------------------|
//! | Windows | `IDesktopWallpaper` (all monitors, Fill) with `SystemParametersInfoW` fallback | WinRT `LockScreen.SetImageFileAsync`; optional PersonalizationCSP policy (admin) |
//! | macOS   | `NSWorkspace.setDesktopImageURL` on every screen (main thread) | follows the desktop picture |
//!
//! Always pass a *new file path* for each change: both OSes cache by path and
//! may ignore a changed file that keeps the same name.

#[cfg(not(windows))]
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

pub mod system;

#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{set_desktop, set_lock_screen, LockScreenMethod};

#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(target_os = "macos")]
pub use macos_impl::set_desktop;

/// Whether this OS lets us set a lock screen image separately from the desktop.
pub const SUPPORTS_SEPARATE_LOCK_SCREEN: bool = cfg!(windows);

#[cfg(not(any(windows, target_os = "macos")))]
pub fn set_desktop(_path: &Path) -> Result<()> {
    Err("setting the wallpaper is not supported on this OS".into())
}

/// macOS / other: the lock screen follows the desktop picture.
#[cfg(not(windows))]
pub fn set_lock_screen(_path: &Path, _allow_policy: bool) -> Result<LockScreenMethod> {
    Err("this OS has no separate lock screen image".into())
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockScreenMethod {
    WinRt,
    Policy,
}
