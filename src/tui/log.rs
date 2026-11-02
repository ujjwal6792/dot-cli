//! `tui.log`: failures and refusals of TUI actions (spec §3). Messages carry
//! paths, line numbers and hints, never secret values.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const CAP: usize = 64 * 1024;

pub fn append(state_dir: &Path, text: &str) {
    let path = state_dir.join("tui.log");
    let _ = fs::create_dir_all(state_dir);
    if fs::metadata(&path).is_ok_and(|m| m.len() as usize > CAP)
        && let Ok(old) = fs::read(&path)
    {
        let old = String::from_utf8_lossy(&old[old.len() - CAP / 2..]).into_owned();
        let tail = old.find('\n').map_or("", |i| &old[i + 1..]);
        let _ = fs::write(&path, tail);
    }
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
    {
        let stamp = crate::pending::now_rfc3339();
        for line in text.lines() {
            let _ = writeln!(f, "{stamp} {line}");
        }
    }
}

pub fn path(state_dir: &Path) -> String {
    state_dir.join("tui.log").display().to_string()
}
