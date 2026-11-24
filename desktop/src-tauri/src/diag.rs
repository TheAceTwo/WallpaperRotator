//! A small diagnostics log (one line per wallpaper change) that the user
//! can open from Settings → Advanced and send when something misbehaves.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// Keep the log small; when it grows past this, keep the newest half.
const MAX_BYTES: u64 = 200 * 1024;

pub fn path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("no data dir").join("diagnostics.log")
}

/// Append `line`, prefixed with the UTC time.
pub fn log(app: &AppHandle, line: &str) {
    let p = path(app);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        if let Ok(text) = std::fs::read_to_string(&p) {
            let keep = &text[text.len() / 2..];
            let start = keep.find('\n').map_or(0, |i| i + 1);
            let _ = std::fs::write(&p, &keep[start..]);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {line}", utc_now());
    }
}

/// "2026-11-24 18:05:09Z" without pulling in a date library.
fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Open the log in the system's text editor.
pub fn open(app: &AppHandle) -> Result<(), String> {
    let p = path(app);
    if !p.exists() {
        log(app, "log created");
    }
    #[cfg(windows)]
    let cmd = std::process::Command::new("notepad.exe").arg(&p).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg("-t").arg(&p).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(&p).spawn();
    cmd.map(|_| ()).map_err(|e| format!("couldn't open the log: {e}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_format_shape() {
        let s = super::utc_now();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z') && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b' ');
    }
}
