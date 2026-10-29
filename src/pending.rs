//! The pending list (spec §8): requested but not yet reviewed operations,
//! persisted in `pending.json` and shared by the CLI and the TUI.
//!
//! Every mutation takes the store's mutation lock; the file is replaced
//! atomically, so readers never need it.

use std::collections::BTreeMap;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::{DwError, Kind};
use crate::fsutil;
use crate::lock::Lock;
use crate::machine::Machine;
use crate::store::Store;

const FORMAT: u32 = 1;

/// A file or symlink a track item writes into the repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrackFile {
    /// Target path relative to HOME.
    pub path: String,
    pub symlink: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Track {
        /// The tracked path (relative to HOME): a file, symlink or folder.
        path: String,
        /// Whether `path` is a folder.
        folder: bool,
        /// Whether the folder is recorded as a tracked folder (false when a
        /// parent folder is already recorded).
        record_folder: bool,
        files: Vec<TrackFile>,
    },
    Untrack {
        path: String,
        /// Tracked files (target paths) removed from the repository.
        files: Vec<String>,
        /// Recorded tracked folders removed.
        forget_folders: Vec<String>,
        /// Assignment keys removed: the paths of the removed files and, for a
        /// folder, the folder path itself and anything below it.
        #[serde(default)]
        assignments: Vec<String>,
        /// Tracked paths whose variants (data and files) are removed too.
        #[serde(default)]
        variants: Vec<String>,
    },
    /// Write the committed version of local changes back to HOME (after a snapshot).
    Discard {
        /// The requested path (relative to HOME): a file or folder.
        path: String,
        /// Files with local changes that are overwritten.
        files: Vec<String>,
    },
    HideAdd {
        pattern: String,
    },
    HideRemove {
        pattern: String,
    },
    RemoteAdd {
        name: String,
        url: String,
    },
    RemoteRemove {
        name: String,
    },
    /// Push the branch to these remotes (every remote when empty).
    Push {
        remotes: Vec<String>,
    },
    /// Set the assignments of tracked paths (spec §4). Each change holds the
    /// final lists; empty lists remove the assignment.
    Assign {
        /// Identifies the request (paths and lists), so a different request for
        /// the same paths is a separate item.
        key: String,
        changes: Vec<AssignChange>,
    },
    MachineAdd {
        machine: Machine,
    },
    MachineRename {
        old: String,
        new: String,
    },
    MachineRetire {
        name: String,
    },
    /// Make the tracked file `path` a variant file (if it is not yet) and store
    /// its HOME content as the variant for `target`.
    VariantAdd {
        path: String,
        target: String,
        /// The variant's file under `home/.dotwarden/variants/`.
        file: String,
        /// Whether the file is not a variant file yet (its stored content becomes
        /// the default variant).
        first: bool,
        /// The current source file of `path`.
        source: String,
    },
    VariantRemove {
        path: String,
        target: String,
        file: String,
        /// The last variant: the file turns back into a plain file.
        last: bool,
        source: String,
    },
    /// Set the default (`target` None) or one override of a variable.
    ValueSet {
        /// Identifies the request (`NAME` or `NAME@target`).
        key: String,
        name: String,
        target: Option<String>,
        value: String,
    },
    /// Remove one override, or (no target) the whole variable.
    ValueUnset {
        key: String,
        name: String,
        target: Option<String>,
    },
    /// Replace this machine's value of a variable in a tracked file by references.
    ValueWire {
        key: String,
        path: String,
        name: String,
    },
    /// Store a tracked file encrypted in the vault (`files/<id>.age`).
    Encrypt {
        key: String,
        path: String,
        id: String,
        /// The current source file of `path`.
        source: String,
    },
    /// Move a value that looks like a secret (found at `line`, byte range
    /// `start..end` of the HOME file; never the value itself) into the vault
    /// as `name` and replace it by a reference.
    SecretMove {
        key: String,
        path: String,
        name: String,
        line: usize,
        start: usize,
        end: usize,
    },
}

/// The final assignment of one target path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssignChange {
    pub path: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Action {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Action::Track { .. } => "track",
            Action::Untrack { .. } => "untrack",
            Action::Discard { .. } => "discard",
            Action::HideAdd { .. } => "hide_add",
            Action::HideRemove { .. } => "hide_remove",
            Action::RemoteAdd { .. } => "remote_add",
            Action::RemoteRemove { .. } => "remote_remove",
            Action::Push { .. } => "push",
            Action::Assign { .. } => "assign",
            Action::MachineAdd { .. } => "machine_add",
            Action::MachineRename { .. } => "machine_rename",
            Action::MachineRetire { .. } => "machine_retire",
            Action::VariantAdd { .. } => "variant_add",
            Action::VariantRemove { .. } => "variant_remove",
            Action::ValueSet { .. } => "value_set",
            Action::ValueUnset { .. } => "value_unset",
            Action::ValueWire { .. } => "value_wire",
            Action::Encrypt { .. } => "encrypt",
            Action::SecretMove { .. } => "secret_move",
        }
    }

    /// Whether only a human may apply this item (spec §8): agents cannot approve
    /// their own requests: discard, push and remote changes.
    pub fn needs_approval(&self) -> bool {
        matches!(
            self,
            Action::Discard { .. }
                | Action::RemoteAdd { .. }
                | Action::RemoteRemove { .. }
                | Action::Push { .. }
                | Action::MachineRetire { .. }
        )
    }

    /// Two items with the same key request the same thing.
    pub fn key(&self) -> (&'static str, &str) {
        let what = match self {
            Action::Track { path, .. }
            | Action::Untrack { path, .. }
            | Action::Discard { path, .. } => path,
            Action::HideAdd { pattern } | Action::HideRemove { pattern } => pattern,
            Action::RemoteAdd { name, .. } | Action::RemoteRemove { name } => name,
            Action::Push { remotes } => remotes.first().map_or("", String::as_str),
            Action::Assign { key, .. } => key,
            Action::MachineAdd { machine } => &machine.name,
            Action::MachineRename { old, .. } => old,
            Action::MachineRetire { name } => name,
            Action::VariantAdd { file, .. } | Action::VariantRemove { file, .. } => file,
            Action::ValueSet { key, .. }
            | Action::ValueUnset { key, .. }
            | Action::ValueWire { key, .. }
            | Action::Encrypt { key, .. }
            | Action::SecretMove { key, .. } => key,
        };
        (self.kind_name(), what)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChange {
    /// `add` or `remove`.
    pub action: String,
    /// The HOME path (relative) this repository file stands for.
    pub target: String,
    /// The repository file (relative to the repository root).
    pub source: String,
    /// `file` or `symlink`.
    pub kind: String,
}

/// Exactly what applying an item changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preview {
    /// Who receives the tracked files ("all machines" for new files).
    pub applies_to: Option<String>,
    pub files: Vec<FileChange>,
    /// Data changes in `.chezmoidata/dotwarden.toml`.
    pub data: Vec<String>,
    /// Hide rule changes.
    pub rules: Vec<String>,
    pub notes: Vec<String>,
    /// Tracked files that start or stop applying on registry machines.
    #[serde(default)]
    pub effects: Vec<String>,
}

/// What an item's outcome depends on. Applying re-computes it and refuses
/// (exit 7, item kept) when it differs.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Guard {
    pub home: BTreeMap<String, String>,
    pub repo: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    /// Assigned when stored; `None` for dry runs and immediate applies.
    pub id: Option<u64>,
    #[serde(flatten)]
    pub action: Action,
    /// `human` or the agent name from `DOTWARDEN_AGENT`.
    pub requester: String,
    /// UTC, RFC 3339.
    pub created: String,
    pub summary: String,
    pub preview: Preview,
    pub guard: Guard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct List {
    pub version: u32,
    pub next_id: u64,
    pub items: Vec<Item>,
}

impl Default for List {
    fn default() -> Self {
        Self {
            version: FORMAT,
            next_id: 1,
            items: Vec::new(),
        }
    }
}

impl List {
    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == Some(id))
    }

    /// The stored item requesting the same thing as `action`, if any.
    pub fn find_same(&self, action: &Action) -> Option<&Item> {
        self.items.iter().find(|i| i.action.key() == action.key())
    }
}

/// Who is asking: the agent named by `DOTWARDEN_AGENT`, else `human`.
pub fn requester(store: &Store) -> String {
    store
        .env()
        .get(std::ffi::OsStr::new("DOTWARDEN_AGENT"))
        .and_then(|v| v.to_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("human")
        .to_string()
}

/// Read the pending list; a missing file is an empty list. Needs no lock.
pub fn load(store: &Store) -> Result<List> {
    let path = store.dirs.pending_file();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(List::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let list: List = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is not a valid pending list; move it away to start with an empty one",
            path.display()
        )
    })?;
    if list.version != FORMAT {
        return Err(DwError::new(
            Kind::Error,
            format!(
                "{} has format {} but this dotwarden understands {FORMAT}",
                path.display(),
                list.version
            ),
        )
        .into());
    }
    Ok(list)
}

/// Persist the list. The lock argument proves the caller holds the mutation lock.
pub fn save(store: &Store, _lock: &Lock, list: &List) -> Result<()> {
    fsutil::write_atomic(
        &store.dirs.pending_file(),
        serde_json::to_string_pretty(list)? + "\n",
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct Added {
    pub item: Item,
    /// True when an identical item was already pending (nothing was added).
    pub already_pending: bool,
    /// True when a stale item for the same thing was replaced by this fresh one
    /// (it keeps its id).
    pub replaced: bool,
}

/// Store new items. An identical pending item is kept, unless `is_stale` says it
/// can no longer be applied: then the fresh item replaces it, keeping its id.
pub fn add(
    store: &Store,
    lock: &Lock,
    items: Vec<Item>,
    is_stale: &dyn Fn(&Item) -> bool,
) -> Result<Vec<Added>> {
    let mut list = load(store)?;
    let mut out = Vec::new();
    for mut item in items {
        if let Some(at) = list
            .items
            .iter()
            .position(|i| i.action.key() == item.action.key())
        {
            if is_stale(&list.items[at]) {
                item.id = list.items[at].id;
                list.items[at] = item.clone();
                out.push(Added {
                    item,
                    already_pending: false,
                    replaced: true,
                });
            } else {
                out.push(Added {
                    item: list.items[at].clone(),
                    already_pending: true,
                    replaced: false,
                });
            }
            continue;
        }
        item.id = Some(list.next_id);
        list.next_id += 1;
        list.items.push(item.clone());
        out.push(Added {
            item,
            already_pending: false,
            replaced: false,
        });
    }
    save(store, lock, &list)?;
    Ok(out)
}

/// Drop items by id. Unknown ids are an error and nothing is dropped.
pub fn drop_items(store: &Store, lock: &Lock, ids: &[u64]) -> Result<Vec<Item>> {
    let mut list = load(store)?;
    for id in ids {
        if list.get(*id).is_none() {
            return Err(not_found(*id));
        }
    }
    let (gone, kept): (Vec<_>, Vec<_>) = list
        .items
        .into_iter()
        .partition(|i| i.id.is_some_and(|id| ids.contains(&id)));
    list.items = kept;
    save(store, lock, &list)?;
    Ok(gone)
}

pub fn not_found(id: u64) -> anyhow::Error {
    DwError::new(Kind::Error, format!("no pending item #{id}")).into()
}

/// Whether HOME path `rel` has a pending track or untrack item: `track`, `untrack`.
pub fn marks(list: &List) -> BTreeMap<String, &'static str> {
    let mut m = BTreeMap::new();
    for i in &list.items {
        match &i.action {
            Action::Track { path, .. } => m.insert(path.clone(), "track"),
            Action::Untrack { path, .. } => m.insert(path.clone(), "untrack"),
            _ => None,
        };
    }
    m
}

impl Item {
    pub fn label(&self) -> String {
        format!("#{}", self.id.map_or("-".to_string(), |i| i.to_string()))
    }

    /// The full preview as text (CLI `pending show`, TUI main view, review dialog).
    pub fn render(&self) -> String {
        let mut s = format!(
            "{} {} — requested by {} at {}\n",
            self.label(),
            self.summary,
            self.requester,
            self.created
        );
        if let Some(a) = &self.preview.applies_to {
            s.push_str(&format!("applies to: {a}\n"));
        }
        let section = |s: &mut String, title: &str, lines: Vec<String>| {
            if !lines.is_empty() {
                s.push_str(&format!("{title}:\n"));
                for l in lines {
                    s.push_str(&format!("  {l}\n"));
                }
            }
        };
        section(
            &mut s,
            "repository files",
            self.preview
                .files
                .iter()
                .map(|f| {
                    format!(
                        "{} {}  ({}, {})",
                        match f.action.as_str() {
                            "add" => "+",
                            "change" => "~",
                            _ => "-",
                        },
                        f.source,
                        f.kind,
                        f.target
                    )
                })
                .collect(),
        );
        section(&mut s, "data", self.preview.data.clone());
        section(&mut s, "hide rules", self.preview.rules.clone());
        const SHOWN: usize = 100;
        let mut effects = self.preview.effects.clone();
        if effects.len() > SHOWN {
            let more = effects.len() - SHOWN;
            effects.truncate(SHOWN);
            effects.push(format!("… {more} more (see `dw pending show --json`)"));
        }
        section(&mut s, "effects on registry machines", effects);
        section(&mut s, "notes", self.preview.notes.clone());
        s
    }
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// `~/rel` for display.
pub fn tilde_rel(rel: &str) -> String {
    format!("~/{rel}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_793_040_207), "2026-10-26T18:43:27Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn items_roundtrip_through_json_with_a_flat_kind() {
        let item = Item {
            id: Some(4),
            action: Action::HideAdd {
                pattern: "x/".into(),
            },
            requester: "human".into(),
            created: rfc3339(0),
            summary: "s".into(),
            preview: Preview::default(),
            guard: Guard::default(),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["kind"], "hide_add");
        assert_eq!(v["pattern"], "x/");
        let back: Item = serde_json::from_value(v).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn tilde_rel_prefixes_home() {
        assert_eq!(tilde_rel(".zshrc"), "~/.zshrc");
    }
}
