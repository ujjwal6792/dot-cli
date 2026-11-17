//! Backup state (spec §6): when this machine last pushed and how many changes
//! no remote holds yet (unbacked changes), for `dw status`, the TUI Status
//! panel and its banner.

use serde::{Deserialize, Serialize};

use crate::changes::{Change, State};
use crate::exec::CancelToken;
use crate::store::Store;

const DAY: u64 = 86_400;

/// The last successful push from this machine, kept in the state directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastPush {
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub remote: String,
    pub branch: String,
}

/// Remember a successful push. Best effort: it never fails the push.
pub fn record_push(store: &Store, remote: &str, branch: &str) {
    let rec = LastPush {
        at: crate::snapshot::now_unix(),
        remote: remote.into(),
        branch: branch.into(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&rec) {
        let _ = crate::fsutil::write_atomic(&store.dirs.last_push_file(), bytes);
    }
}

pub fn last_push(store: &Store) -> Option<LastPush> {
    let bytes = std::fs::read(store.dirs.last_push_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Backup {
    /// When this machine last pushed (RFC 3339); `None` if it never did.
    pub last_pushed: Option<String>,
    pub last_pushed_days: Option<u64>,
    /// Local and New files, staged paths and commits not on the upstream.
    pub unbacked: usize,
    pub unbacked_commits: usize,
    /// Whole days the oldest unbacked change has waited, counted from the last
    /// push (or, never pushed, from the oldest commit no remote holds).
    pub waiting_days: Option<u64>,
    pub warn_days: u64,
    /// Unbacked changes have waited longer than `warn_days`.
    pub warning: bool,
    /// `last pushed 9 days ago · 4 unbacked`; `None` when everything is pushed.
    pub summary: Option<String>,
}

impl Backup {
    /// The status line: the summary, with the advice when it is a warning.
    pub fn line(&self) -> Option<String> {
        let s = self.summary.as_ref()?;
        Some(if self.warning {
            format!("{s} — run dw sync")
        } else {
            s.clone()
        })
    }
}

fn git_text(store: &Store, args: &[&str], cancel: Option<&CancelToken>) -> Option<String> {
    let out = store.git_cancellable(args, cancel).ok()?;
    Some(out.stdout.trim().to_string())
}

fn days_text(days: Option<u64>) -> String {
    match days {
        None => "never pushed".into(),
        Some(0) => "last pushed today".into(),
        Some(1) => "last pushed 1 day ago".into(),
        Some(n) => format!("last pushed {n} days ago"),
    }
}

/// The backup state from the classified `changes` and the branch's `upstream`.
pub fn collect(
    store: &Store,
    changes: &[Change],
    upstream: Option<&str>,
    cancel: Option<&CancelToken>,
) -> Backup {
    let now = crate::snapshot::now_unix();
    let files = changes
        .iter()
        .filter(|c| matches!(c.state, State::Local | State::New | State::Staged))
        .count();
    let range: Vec<String> = match upstream {
        Some(u) => vec![format!("{u}..HEAD")],
        None => vec!["HEAD".into(), "--not".into(), "--remotes".into()],
    };
    let mut log = vec!["log", "--format=%ct"];
    log.extend(range.iter().map(String::as_str));
    let times: Vec<u64> = git_text(store, &log, cancel)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.parse().ok())
        .collect();
    let last = last_push(store);
    let since = last
        .as_ref()
        .map(|l| l.at)
        .or_else(|| times.iter().min().copied());
    let unbacked = files + times.len();
    let warn_days = crate::prefs::Prefs::load(store).backup_warn_days();
    let waited = since.map(|s| now.saturating_sub(s));
    let warning = unbacked > 0 && waited.is_some_and(|w| w > warn_days * DAY);
    let last_pushed_days = last.as_ref().map(|l| now.saturating_sub(l.at) / DAY);
    Backup {
        last_pushed: last.as_ref().map(|l| crate::pending::rfc3339(l.at)),
        last_pushed_days,
        unbacked,
        unbacked_commits: times.len(),
        waiting_days: waited.map(|w| w / DAY),
        warn_days,
        warning,
        summary: (unbacked > 0)
            .then(|| format!("{} · {unbacked} unbacked", days_text(last_pushed_days))),
    }
}
