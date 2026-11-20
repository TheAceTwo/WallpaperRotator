//! Where to put the tray flyout (pure math, unit-tested).
//!
//! The settings window opens like a system flyout: next to the tray icon,
//! inside the screen's work area (the part not covered by the taskbar /
//! menu bar), above the icon when the icon is at the bottom of the screen
//! (Windows taskbar) and below it when it's at the top (macOS menu bar or a
//! top taskbar).

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Gap between the flyout and the screen edges / taskbar.
pub const MARGIN: i32 = 8;

/// Returns `(x, y, height)` for a `win_w` × `win_h` flyout.
/// `icon` is the tray icon's rectangle, if known; without it the flyout goes
/// to the bottom-right corner (where the Windows tray normally is).
pub fn place(icon: Option<Area>, work: Area, win_w: u32, win_h: u32) -> (i32, i32, u32) {
    let max_h = (work.h as i32 - 2 * MARGIN).max(100) as u32;
    let h = win_h.min(max_h);
    let left = work.x + MARGIN;
    let right = work.x + work.w as i32 - win_w as i32 - MARGIN;
    let top = work.y + MARGIN;
    let bottom = work.y + work.h as i32 - h as i32 - MARGIN;

    let Some(icon) = icon else {
        return (right.max(left), bottom.max(top), h);
    };
    let icon_cx = icon.x + icon.w as i32 / 2;
    let icon_cy = icon.y + icon.h as i32 / 2;
    let x = (icon_cx - win_w as i32 / 2).clamp(left, right.max(left));
    let work_cy = work.y + work.h as i32 / 2;
    let y = if icon_cy >= work_cy { bottom } else { top };
    (x, y.max(top), h)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920x1080 screen with a 48 px taskbar at the bottom.
    const WORK: Area = Area { x: 0, y: 0, w: 1920, h: 1032 };

    #[test]
    fn bottom_taskbar_opens_above_icon() {
        let icon = Area { x: 1500, y: 1040, w: 32, h: 32 };
        let (x, y, h) = place(Some(icon), WORK, 400, 680);
        assert_eq!(h, 680);
        assert_eq!(x, 1516 - 200); // centred on the icon
        assert_eq!(y, 1032 - 680 - MARGIN); // just above the taskbar
    }

    #[test]
    fn clamped_at_right_edge() {
        let icon = Area { x: 1880, y: 1040, w: 32, h: 32 };
        let (x, _, _) = place(Some(icon), WORK, 400, 680);
        assert_eq!(x, 1920 - 400 - MARGIN);
    }

    #[test]
    fn top_menu_bar_opens_below_icon() {
        // macOS: 25 px menu bar at the top.
        let work = Area { x: 0, y: 25, w: 1440, h: 875 };
        let icon = Area { x: 1200, y: 0, w: 24, h: 24 };
        let (x, y, h) = place(Some(icon), work, 400, 680);
        assert_eq!((x, y, h), (1212 - 200, 25 + MARGIN, 680));
    }

    #[test]
    fn small_screen_caps_height() {
        // 600 px of usable height: a 680 px flyout must shrink to fit.
        let work = Area { x: 0, y: 0, w: 1024, h: 600 };
        let (_, y, h) = place(None, work, 400, 680);
        assert_eq!(h, 600 - 2 * MARGIN as u32);
        assert_eq!(y, MARGIN);
    }

    #[test]
    fn unknown_icon_goes_bottom_right() {
        let (x, y, _) = place(None, WORK, 400, 680);
        assert_eq!((x, y), (1920 - 400 - MARGIN, 1032 - 680 - MARGIN));
    }

    #[test]
    fn second_monitor_offsets() {
        // Monitor to the right of the main one.
        let work = Area { x: 1920, y: 0, w: 2560, h: 1392 };
        let icon = Area { x: 4300, y: 1400, w: 32, h: 32 };
        let (x, y, _) = place(Some(icon), work, 400, 680);
        assert_eq!(x, 1920 + 2560 - 400 - MARGIN);
        assert_eq!(y, 1392 - 680 - MARGIN);
    }
}
