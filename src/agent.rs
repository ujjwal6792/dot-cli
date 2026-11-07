//! Who is acting (spec §8) and the local activity log.
//!
//! The activity log records which command ran, who ran it and how it ended.
//! Never arguments, paths or values, so it cannot leak a secret.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::paths::{Dirs, Env};

/// Agent environments recognised without `DOTWARDEN_AGENT`: (variable, name).
/// A variable counts when it is set to something other than empty, `0` or `false`.
const KNOWN: &[(&str, &str)] = &[
    ("CLAUDECODE", "claude-code"),
    ("OPENCODE", "opencode"),
    ("CODEX_SANDBOX", "codex"),
    ("CODEX_THREAD_ID", "codex"),
    ("CURSOR_AGENT", "cursor"),
    ("GEMINI_CLI", "gemini"),
];

/// Lines kept when the log grows past [`MAX_BYTES`].
const KEEP_LINES: usize = 1000;
const MAX_BYTES: u64 = 256 * 1024;

fn var<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.get(std::ffi::OsStr::new(name))
        .and_then(|v| v.to_str())
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "0" && !v.eq_ignore_ascii_case("false"))
}

/// The agent acting, or `None` for a human. `DOTWARDEN_AGENT` always counts
/// (`AI_AGENT` names itself). A known agent environment counts only when this
/// is not an interactive terminal, so a person who opens a terminal inside an
/// agent's tool still acts as themselves.
pub fn name(env: &Env) -> Option<String> {
    use std::io::IsTerminal;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    name_for(env, interactive)
}

fn name_for(env: &Env, interactive: bool) -> Option<String> {
    var(env, "DOTWARDEN_AGENT")
        .or_else(|| var(env, "AI_AGENT"))
        .map(str::to_string)
        .or_else(|| {
            if interactive {
                return None;
            }
            KNOWN
                .iter()
                .find(|(v, _)| var(env, v).is_some())
                .map(|(_, n)| (*n).to_string())
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub time: String,
    /// `human` or the agent name.
    pub actor: String,
    /// Subcommand path, e.g. `pending apply`.
    pub command: String,
    pub exit: u8,
}

/// Append one record. Best effort: a log that cannot be written never fails a command.
pub fn record(dirs: &Dirs, env: &Env, command: &str, exit: u8) {
    let entry = Entry {
        time: crate::pending::now_rfc3339(),
        actor: name(env).unwrap_or_else(|| "human".into()),
        command: command.to_string(),
        exit,
    };
    let _ = append(&dirs.activity_file(), &entry);
}

fn append(path: &Path, entry: &Entry) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let text = fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let kept = lines[lines.len().saturating_sub(KEEP_LINES)..].join("\n") + "\n";
        fsutil::write_atomic(path, kept)?;
    }
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

/// The newest `count` records, newest last; unreadable lines are skipped.
pub fn read(dirs: &Dirs, count: usize, actor: Option<&str>) -> Result<Vec<Entry>> {
    let text = match fs::read_to_string(dirs.activity_file()) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading the activity log"),
    };
    let mut all: Vec<Entry> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|e: &Entry| actor.is_none_or(|a| e.actor == a))
        .collect();
    let skip = all.len().saturating_sub(count);
    Ok(all.split_off(skip))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    #[test]
    fn explicit_name_beats_known_environments() {
        let e = env(&[("DOTWARDEN_AGENT", "mine"), ("CLAUDECODE", "1")]);
        assert_eq!(name_for(&e, true).as_deref(), Some("mine"));
        let claude = env(&[("CLAUDECODE", "1")]);
        assert_eq!(name_for(&claude, false).as_deref(), Some("claude-code"));
        assert_eq!(name_for(&claude, true), None);
        assert_eq!(name_for(&env(&[("CLAUDECODE", "0")]), false), None);
        assert_eq!(name_for(&env(&[]), false), None);
    }
}
