use crate::Result;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSString, NSURL};
use std::path::Path;

/// Set the desktop picture on every connected screen.
///
/// Must be called on the main thread (AppKit requirement). The macOS lock
/// screen shows the desktop picture, so this covers both.
///
/// Limitation: macOS applies this to the *current* Space on each screen.
pub fn set_desktop(path: &Path) -> Result<()> {
    set_desktop_split(path, path)
}

/// Number of connected screens (1 if not on the main thread).
pub fn monitor_count() -> usize {
    MainThreadMarker::new().map_or(1, |mtm| NSScreen::screens(mtm).len().max(1))
}

/// `primary` on the main screen (the one with the menu bar), `others` elsewhere.
/// Must be called on the main thread.
pub fn set_desktop_split(primary: &Path, others: &Path) -> Result<()> {
    let mtm = MainThreadMarker::new().ok_or("set_desktop must run on the main thread")?;
    let to_url = |p: &Path| -> Result<objc2::rc::Retained<NSURL>> {
        let s = p.to_str().ok_or("wallpaper path is not valid UTF-8")?;
        Ok(NSURL::fileURLWithPath(&NSString::from_str(s)))
    };
    let (primary_url, others_url) = (to_url(primary)?, to_url(others)?);
    let workspace = NSWorkspace::sharedWorkspace();

    let screens = NSScreen::screens(mtm);
    if screens.is_empty() {
        return Err("no screens found".into());
    }
    for (i, screen) in screens.iter().enumerate() {
        // screens[0] is always the main screen.
        let url = if i == 0 { &primary_url } else { &others_url };
        // Keep the user's current scaling/fill options for that screen.
        let options = workspace
            .desktopImageOptionsForScreen(&screen)
            .unwrap_or_else(NSDictionary::new);
        // SAFETY: `options` is a dictionary returned by AppKit (or empty), so
        // its keys/values are the types the API expects.
        unsafe { workspace.setDesktopImageURL_forScreen_options_error(&url, &screen, &options) }
            .map_err(|e| format!("macOS refused the wallpaper: {}", e.localizedDescription()))?;
    }
    Ok(())
}
