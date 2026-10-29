//! Background job runner: all I/O happens on worker threads, results come back
//! as events, and a newer job with the same key cancels and supersedes the old.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::thread;

use super::event::Event;
use super::files::Entry;
use crate::exec::CancelToken;
use crate::hide::{HideRules, HideStamp};
use crate::pending::List;
use crate::source::Tracked;
use crate::status::Status;
use crate::track::ItemResult;

/// Job keys; at most one job per key is current.
pub const KEY_STATUS: &str = "status";
pub const KEY_LISTING: &str = "listing";
pub const KEY_PREVIEW: &str = "preview";
pub const KEY_TRACKED: &str = "tracked";
pub const KEY_PENDING: &str = "pending";
pub const KEY_HIDE: &str = "hide";
/// Mutations never supersede one another: every one runs, in the order requested.
pub const KEY_DIFF: &str = "diff";
pub const KEY_ACTION: &str = "action";
/// The background fetch; a newer one cancels an older.
pub const KEY_FETCH: &str = "fetch";
pub const KEY_REGISTRY: &str = "registry";
pub const KEY_MACHINE: &str = "machine";
pub const KEY_VALUES: &str = "values";

/// The Values screen's data, read off the event loop.
#[derive(Debug, Default)]
pub struct ValuesView {
    pub rows: Vec<crate::values::Row>,
    /// Decrypted secrets; empty while the vault is locked.
    pub reveal: std::collections::BTreeMap<String, crate::values::Reveal>,
}

/// The machine registry and the assignments, as the TUI shows them.
#[derive(Debug, Default, Clone)]
pub struct RegistryView {
    pub machines: Vec<crate::machines::Listed>,
    pub assignments: crate::assign::Assignments,
}

/// The outcome of a mutation (toggle, drop, apply).
#[derive(Debug)]
pub struct ActionReport {
    pub message: String,
    /// Per-item results of an apply.
    pub results: Option<Vec<ItemResult>>,
    /// The last commit message, to open the amend box with.
    pub amend_message: Option<String>,
}

#[derive(Debug)]
pub enum Payload {
    Status(Result<Box<Status>, String>),
    Listing {
        dir: PathBuf,
        entries: Result<Vec<Entry>, String>,
    },
    Preview {
        path: PathBuf,
        text: Result<String, String>,
    },
    Tracked(Result<Box<Tracked>, String>),
    /// The hide rules as loaded, with the change marker of the `hide` file.
    Hide(Result<(Box<HideRules>, HideStamp), String>),
    Pending(Result<Box<List>, String>),
    /// The diff of the selected change, styled by delta when available.
    Diff(Result<ratatui::text::Text<'static>, String>),
    Action(Result<ActionReport, String>),
    /// A background fetch: `None` when there was nothing to fetch from.
    Fetch(Result<Option<String>, String>),
    Registry(Result<Box<RegistryView>, String>),
    Values(Result<Box<ValuesView>, String>),
    /// What a registry machine would receive.
    MachinePreview {
        name: String,
        text: Result<String, String>,
    },
}

#[derive(Debug)]
pub struct Done {
    pub id: u64,
    pub key: &'static str,
    pub payload: Payload,
}

pub struct Jobs {
    tx: Sender<Event>,
    next_id: u64,
    current: HashMap<&'static str, (u64, CancelToken)>,
    in_flight: Arc<AtomicUsize>,
}

impl Jobs {
    pub fn new(tx: Sender<Event>) -> Self {
        Self {
            tx,
            next_id: 0,
            current: HashMap::new(),
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Start `work` on a worker thread, cancelling any earlier job with `key`.
    pub fn spawn<F>(&mut self, key: &'static str, work: F) -> u64
    where
        F: FnOnce(&CancelToken) -> Payload + Send + 'static,
    {
        self.start(key, work, true)
    }

    /// Start `work` without superseding or being superseded by any other job.
    pub fn spawn_detached<F>(&mut self, key: &'static str, work: F) -> u64
    where
        F: FnOnce(&CancelToken) -> Payload + Send + 'static,
    {
        self.start(key, work, false)
    }

    fn start<F>(&mut self, key: &'static str, work: F, supersede: bool) -> u64
    where
        F: FnOnce(&CancelToken) -> Payload + Send + 'static,
    {
        self.next_id += 1;
        let id = self.next_id;
        let token = CancelToken::new();
        if supersede && let Some((_, old)) = self.current.insert(key, (id, token.clone())) {
            old.cancel();
        }
        let tx = self.tx.clone();
        let in_flight = Arc::clone(&self.in_flight);
        in_flight.fetch_add(1, Ordering::SeqCst);
        thread::spawn(move || {
            let payload = work(&token);
            in_flight.fetch_sub(1, Ordering::SeqCst);
            if !token.is_cancelled() {
                let _ = tx.send(Event::Job(Done { id, key, payload }));
            }
        });
        id
    }

    /// Whether `id` is still the newest job for `key` (otherwise its result is stale).
    pub fn is_current(&self, key: &str, id: u64) -> bool {
        key == KEY_ACTION || self.current.get(key).is_some_and(|(cur, _)| *cur == id)
    }

    /// Jobs whose worker has not returned yet.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        for (_, token) in self.current.values() {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn status_payload() -> Payload {
        Payload::Status(Err("x".into()))
    }

    #[test]
    fn newer_job_supersedes_older() {
        let (tx, rx) = channel();
        let mut jobs = Jobs::new(tx);
        let first = jobs.spawn(KEY_STATUS, |t| {
            while !t.is_cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
            status_payload()
        });
        let second = jobs.spawn(KEY_STATUS, |_| status_payload());
        assert!(!jobs.is_current(KEY_STATUS, first));
        assert!(jobs.is_current(KEY_STATUS, second));
        match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
            Event::Job(d) => assert_eq!(d.id, second),
            other => panic!("unexpected {other:?}"),
        }
        // The cancelled job's result is never delivered.
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn spawn_returns_immediately_while_work_runs() {
        let (tx, _rx) = channel();
        let mut jobs = Jobs::new(tx);
        let start = std::time::Instant::now();
        jobs.spawn(KEY_PREVIEW, |_| {
            thread::sleep(Duration::from_millis(300));
            status_payload()
        });
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(jobs.in_flight(), 1);
    }
}
