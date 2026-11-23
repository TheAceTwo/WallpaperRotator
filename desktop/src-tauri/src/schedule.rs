//! Pure decision logic for the photo cache and change triggers (unit-tested).

use immich_core::Orientation;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many photos to keep downloaded ahead of time.
pub const QUEUE_LEN: usize = 3;

/// A photo downloaded ahead of time so the next change is instant/offline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueuedPhoto {
    pub asset_id: String,
    pub path: PathBuf,
    /// Settings it was picked for; a mismatch means it's stale.
    pub share_url: String,
    pub orientation: Orientation,
}

/// Remove queued photos that no longer match the settings or whose file is
/// gone. Returns the files to delete.
pub fn prune_queue(
    queue: &mut Vec<QueuedPhoto>,
    share_url: &str,
    orientation: Orientation,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    queue.retain(|q| {
        let keep = q.share_url == share_url && q.orientation == orientation && exists(&q.path);
        if !keep {
            removed.push(q.path.clone());
        }
        keep
    });
    removed
}

/// Take up to `n` photos from the front of the queue (oldest first).
pub fn take(queue: &mut Vec<QueuedPhoto>, n: usize) -> Vec<QueuedPhoto> {
    queue.drain(..n.min(queue.len())).collect()
}

/// Is this the first launch since the computer started / the user signed in?
///
/// `previous` is the session start recorded at the last launch; `current` is
/// now's. Small differences are clock jitter. If the session start can't be
/// determined, assume a fresh start (the app normally launches at sign-in).
pub fn is_new_session(previous: Option<u64>, current: Option<u64>) -> bool {
    match (previous, current) {
        (Some(p), Some(c)) => p.abs_diff(c) > 60,
        (_, None) | (None, Some(_)) => true,
    }
}

/// Seconds until the next timed change, given the last change time.
pub fn secs_until_due(last_changed: Option<u64>, interval_minutes: u32, now: u64) -> u64 {
    let interval = u64::from(interval_minutes.max(1)) * 60;
    match last_changed {
        Some(last) if last <= now => (last + interval).saturating_sub(now),
        _ => 0, // never changed (or clock went backwards): due now
    }
}

/// Links to try, in order: the main link, then the backup (if enabled and
/// different), except that whichever worked last time goes first.
pub fn link_order(main: &str, backup: Option<&str>, last_good: Option<&str>) -> Vec<String> {
    let mut urls = vec![main.trim().to_string()];
    if let Some(b) = backup.map(str::trim).filter(|b| !b.is_empty() && *b != urls[0]) {
        urls.push(b.to_string());
    }
    if let Some(i) = last_good.and_then(|g| urls.iter().position(|u| u == g)) {
        urls.swap(0, i);
    }
    urls
}

/// Most virtual desktops that get their own photo (more just cycle).
pub const MAX_DESKTOP_PHOTOS: usize = 10;
/// macOS can't count Spaces publicly; this many extra photos are prepared
/// per change and handed out as you visit other Spaces.
pub const MAC_SPACE_PHOTOS: usize = 3;

/// How many photos *besides* the main one each change needs for
/// "different photo on each virtual desktop / Space".
pub fn extra_desktop_photos(enabled: bool, windows_desktops: Option<usize>, is_mac: bool) -> usize {
    if !enabled {
        return 0;
    }
    if is_mac {
        return MAC_SPACE_PHOTOS;
    }
    match windows_desktops {
        Some(n) if n > 1 => n.min(MAX_DESKTOP_PHOTOS) - 1,
        _ => 0, // one desktop, or not supported on this Windows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(id: &str, url: &str, o: Orientation) -> QueuedPhoto {
        QueuedPhoto { asset_id: id.into(), path: PathBuf::from(format!("/q/{id}.jpg")), share_url: url.into(), orientation: o }
    }

    #[test]
    fn prune_drops_stale_and_missing() {
        let mut queue = vec![
            q("a", "u1", Orientation::Landscape),
            q("b", "u2", Orientation::Landscape),     // other link
            q("c", "u1", Orientation::Portrait),      // other orientation
            q("d", "u1", Orientation::Landscape),     // file missing
            q("e", "u1", Orientation::Landscape),
        ];
        let removed = prune_queue(&mut queue, "u1", Orientation::Landscape, |p| !p.ends_with("d.jpg"));
        assert_eq!(queue.iter().map(|x| x.asset_id.as_str()).collect::<Vec<_>>(), ["a", "e"]);
        assert_eq!(removed.len(), 3);
    }

    #[test]
    fn take_is_fifo_and_bounded() {
        let mut queue = vec![q("a", "u", Orientation::Any), q("b", "u", Orientation::Any)];
        assert_eq!(take(&mut queue, 1)[0].asset_id, "a");
        assert_eq!(take(&mut queue, 5).len(), 1);
        assert!(take(&mut queue, 1).is_empty());
    }

    #[test]
    fn extra_photo_counts() {
        assert_eq!(extra_desktop_photos(false, Some(4), false), 0, "setting off");
        assert_eq!(extra_desktop_photos(true, Some(4), false), 3, "4 desktops -> 3 extra");
        assert_eq!(extra_desktop_photos(true, Some(1), false), 0, "one desktop");
        assert_eq!(extra_desktop_photos(true, None, false), 0, "unsupported Windows");
        assert_eq!(extra_desktop_photos(true, Some(25), false), MAX_DESKTOP_PHOTOS - 1, "capped");
        assert_eq!(extra_desktop_photos(true, None, true), MAC_SPACE_PHOTOS, "macOS");
    }

    #[test]
    fn link_order_rules() {
        let (m, b) = ("https://public/share/k", "http://100.64.0.5:2283/share/k");
        assert_eq!(link_order(m, None, None), [m]);
        assert_eq!(link_order(m, Some(b), None), [m, b], "main first by default");
        assert_eq!(link_order(m, Some(b), Some(b)), [b, m], "last working link first");
        assert_eq!(link_order(m, Some("  "), None), [m], "empty backup ignored");
        assert_eq!(link_order(m, Some(m), None), [m], "same link not tried twice");
        assert_eq!(link_order(m, None, Some(b)), [m], "backup turned off");
    }

    #[test]
    fn new_session_detection() {
        assert!(is_new_session(None, Some(1000)), "first ever launch");
        assert!(is_new_session(Some(1000), Some(5000)), "signed in again");
        assert!(!is_new_session(Some(1000), Some(1030)), "same session, jitter");
        assert!(is_new_session(Some(1000), None), "unknown -> treat as fresh");
    }

    #[test]
    fn timer_continues_from_last_change() {
        // 60-minute interval, changed 20 minutes ago -> 40 minutes left.
        assert_eq!(secs_until_due(Some(10_000), 60, 10_000 + 20 * 60), 40 * 60);
        assert_eq!(secs_until_due(Some(10_000), 60, 10_000 + 90 * 60), 0);
        assert_eq!(secs_until_due(None, 60, 10_000), 0);
        assert_eq!(secs_until_due(Some(20_000), 60, 10_000), 0, "clock went backwards");
    }
}
