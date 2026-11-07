//! The undo journal (spec §11): `z` takes back the last local action.
//!
//! The repository always equals its index, so a source-side action (stage,
//! unstage, track, untrack, assign, commit, ...) is fully described by the
//! `(HEAD, index tree)` pair before and after it. Undo is allowed only while
//! the repository still is in the recorded "after" state, so it can never
//! clobber something it did not record (a pull, a hand edit). A discard is
//! undone from its snapshot. Nothing here ever pushes.

use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::error::{DwError, Kind};
use crate::fsutil;
use crate::pending::now_rfc3339;
use crate::store::Store;
use crate::track::{ensure_repo_clean, index_tree, refuse, rollback};

/// How many actions the journal remembers.
pub const KEEP: usize = 20;

/// `(HEAD, index tree)` of the repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub head: String,
    pub tree: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// `stage`, `unstage`, `track`, `untrack`, `assign`, `commit`, `discard`, ...
    pub kind: String,
    pub summary: String,
    pub actor: String,
    pub created: String,
    /// Repository state around the action; `None` for a discard.
    pub before: Option<Span>,
    pub after: Option<Span>,
    /// Snapshot holding what a discard removed.
    pub snapshot: Option<String>,
    /// Copy of chezmoi's state from before the action (`undo/` in the state directory).
    #[serde(default)]
    pub state: Option<String>,
}

/// The repository, and chezmoi's state, before an action.
pub struct Mark {
    span: Span,
    state: Option<Vec<u8>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Journal {
    entries: Vec<Entry>,
}

fn load(store: &Store) -> Journal {
    fs::read(store.dirs.undo_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(store: &Store, journal: &Journal) -> Result<()> {
    fsutil::write_atomic(&store.dirs.undo_file(), serde_json::to_vec_pretty(journal)?)
}

fn blob_dir(store: &Store) -> std::path::PathBuf {
    store.dirs.state_dir().join("undo")
}

fn push(store: &Store, entry: Entry) {
    let mut journal = load(store);
    journal.entries.push(entry);
    let extra = journal.entries.len().saturating_sub(KEEP);
    for old in journal.entries.drain(..extra) {
        forget_blob(store, &old);
    }
    // Recording is best effort: it must never fail the action it follows.
    let _ = save(store, &journal);
}

fn forget_blob(store: &Store, entry: &Entry) {
    if let Some(name) = &entry.state {
        let _ = fs::remove_file(blob_dir(store).join(name));
    }
}

fn span(store: &Store) -> Result<Span> {
    let head = store.git(&["rev-parse", "HEAD"])?.stdout.trim().to_string();
    Ok(Span {
        head,
        tree: index_tree(store)?,
    })
}

/// The state before an action; call with the mutation lock held.
pub fn mark(store: &Store) -> Option<Mark> {
    Some(Mark {
        span: span(store).ok()?,
        state: crate::track::state_backup(store).ok()?,
    })
}

/// Journal the action that began at `before`. Nothing is recorded when the
/// repository did not change.
pub fn record(store: &Store, before: Option<Mark>, kind: &str, summary: String, actor: &str) {
    let (Some(before), Ok(after)) = (before, span(store)) else {
        return;
    };
    if before.span == after {
        return;
    }
    let state = before.state.and_then(|bytes| {
        let name = format!("{}.state", crate::snapshot::now_unix_nanos());
        fs::create_dir_all(blob_dir(store)).ok()?;
        fsutil::write_atomic(&blob_dir(store).join(&name), bytes).ok()?;
        Some(name)
    });
    push(
        store,
        Entry {
            kind: kind.into(),
            summary,
            actor: actor.into(),
            created: now_rfc3339(),
            before: Some(before.span),
            after: Some(after),
            snapshot: None,
            state,
        },
    );
}

/// Journal a discard whose files were preserved in `snapshot`.
pub fn record_discard(store: &Store, snapshot: Option<String>, files: &[String], actor: &str) {
    let Some(snapshot) = snapshot else { return };
    push(
        store,
        Entry {
            kind: "discard".into(),
            summary: describe("discard", files),
            actor: actor.into(),
            created: now_rfc3339(),
            before: None,
            after: None,
            snapshot: Some(snapshot),
            state: None,
        },
    );
}

/// "stage ~/.zshrc" or "stage 3 files".
pub fn describe(verb: &str, paths: &[String]) -> String {
    match paths {
        [one] => format!("{verb} ~/{one}"),
        many => format!("{verb} {} files", many.len()),
    }
}

/// The name journal entries are recorded under: `human` or the agent.
pub fn actor(store: &Store) -> String {
    crate::changes::agent(store).unwrap_or_else(|| "human".into())
}

/// The recorded actions, newest last.
pub fn list(store: &Store) -> Vec<Entry> {
    load(store).entries
}

#[derive(Debug, Serialize)]
pub struct Undone {
    pub kind: String,
    pub summary: String,
    pub dry_run: bool,
    /// What the undo does, one line each.
    pub effects: Vec<String>,
}

fn stale(why: String) -> anyhow::Error {
    DwError::new(Kind::Stale, why).into()
}

/// Take back the newest journalled action.
pub fn undo(store: &Store, dry_run: bool) -> Result<Undone> {
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    let mut journal = load(store);
    let Some(entry) = journal.entries.last().cloned() else {
        return Err(refuse("there is nothing to undo".into()));
    };
    let mut effects = Vec::new();
    match (&entry.before, &entry.after, &entry.snapshot) {
        (Some(before), Some(after), _) => {
            let now = span(store)?;
            if now != *after {
                return Err(stale(format!(
                    "the repository changed since \"{}\" (a pull, a hand edit or an action \
                     that is not journalled); nothing was undone",
                    entry.summary
                )));
            }
            ensure_repo_clean(store)?;
            let commit = before.head != after.head;
            if commit {
                crate::changes::ensure_amendable(store).map_err(|e| {
                    refuse(format!("\"{}\" cannot be undone: {e:#}", entry.summary))
                })?;
                effects.push(format!(
                    "move HEAD back to {} (the commit's changes stay staged)",
                    &before.head[..before.head.len().min(8)]
                ));
            }
            if before.tree != after.tree {
                effects.push("restore the repository and index to how they were".into());
            }
            if !dry_run {
                if commit {
                    store.git(&["reset", "--soft", "--quiet", before.head.as_str()])?;
                }
                if before.tree != after.tree {
                    rollback(store, &before.tree)?;
                }
                if let Some(name) = &entry.state
                    && let Ok(bytes) = fs::read(blob_dir(store).join(name))
                {
                    crate::track::state_restore(store, Some(bytes))?;
                }
            }
        }
        (_, _, Some(snapshot)) => {
            let snap = crate::snapshot::find(store, snapshot)?;
            for f in &snap.files {
                effects.push(format!("put ~/{} back from snapshot {snapshot}", f.path));
            }
            if !dry_run {
                crate::snapshot::restore(store, snapshot, &[], &actor(store))?;
            }
        }
        _ => return Err(refuse("the journal entry is unusable".into())),
    }
    if !dry_run {
        journal.entries.pop();
        forget_blob(store, &entry);
        save(store, &journal)?;
    }
    Ok(Undone {
        kind: entry.kind,
        summary: entry.summary,
        dry_run,
        effects,
    })
}
