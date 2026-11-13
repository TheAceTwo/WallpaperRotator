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
    let mtm = MainThreadMarker::new().ok_or("set_desktop must run on the main thread")?;
    let path = path.to_str().ok_or("wallpaper path is not valid UTF-8")?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let workspace = NSWorkspace::sharedWorkspace();

    let screens = NSScreen::screens(mtm);
    if screens.is_empty() {
        return Err("no screens found".into());
    }
    for screen in screens.iter() {
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
