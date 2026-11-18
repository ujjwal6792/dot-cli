//! `dw import` (spec §5a, ADR 0008): an existing setup's file list becomes
//! track items for the files as they are in HOME now. Import is a front end to
//! `track`: it only decides which paths to track (folder collapsing) and records
//! where they came from.
//!
//! "While staged changes come from import items" is kept in `import.json` in
//! the state directory: applying an item with a source records its label and
//! target paths there, the next commit removes the file, and a commit message is
//! prefixed only while one of the recorded paths is staged.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::DwError;
use crate::exec;
use crate::pending::{Action, Item, tilde_rel};
use crate::store::Store;
use crate::track::{self, Ctx, Plan, Planned, Skipped};

/// Where import items came from: shown in the item and the commit message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    /// The bare repository or list file (`~/…` below HOME), or `stdin`.
    pub from: String,
    /// The bare repository's HEAD as a short hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

impl Source {
    /// `Import from <DIR> at <hash>`, or `Import from <LIST>`.
    pub fn label(&self) -> String {
        match &self.head {
            Some(h) => format!("Import from {} at {h}", self.from),
            None => format!("Import from {}", self.from),
        }
    }
}

/// The setup to read: `--bare DIR` or `--from-file LIST` (`-` is stdin).
#[derive(Debug, Clone)]
pub enum From {
    Bare(String),
    List(String),
}

/// How the listed files are tracked.
#[derive(Debug, Default, Serialize)]
pub struct Grouping {
    /// Tracked folders with the number of listed files in each.
    pub folders: Vec<GroupFolder>,
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupFolder {
    pub path: String,
    pub files: usize,
}

pub struct ImportPlan {
    pub source: Source,
    /// Paths read from the source.
    pub listed: usize,
    pub grouping: Grouping,
    pub planned: Planned,
}

fn shown(store: &Store, p: &Path) -> String {
    match p.strip_prefix(&store.dirs.home) {
        Ok(r) if !r.as_os_str().is_empty() => format!("~/{}", r.display()),
        _ => p.display().to_string(),
    }
}

fn absolute(store: &Store, arg: &str, cwd: &Path) -> PathBuf {
    if arg == "~" {
        store.dirs.home.clone()
    } else if let Some(r) = arg.strip_prefix("~/") {
        store.dirs.home.join(r)
    } else {
        cwd.join(arg)
    }
}

/// Run the bundled git read-only against the old repository with HOME as its
/// work tree. Nothing is written there: `ls-files` and `rev-parse` never
/// refresh the index, and optional locks are off.
fn old_git(store: &Store, dir: &Path, args: &[&str]) -> Result<exec::Output> {
    let mut cmd = store.git_command(&store.dirs.home);
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd.arg(format!("--git-dir={}", dir.display()));
    cmd.arg(format!("--work-tree={}", store.dirs.home.display()));
    cmd.args(args);
    exec::run(cmd, None)
}

fn read_bare(store: &Store, arg: &str, cwd: &Path) -> Result<(Source, Vec<String>)> {
    let dir = absolute(store, arg, cwd);
    let from = shown(store, &dir);
    if !dir.is_dir() || !old_git(store, &dir, &["rev-parse", "--git-dir"])?.success() {
        return Err(DwError::usage(format!("{from} is not a git repository")).into());
    }
    let head = old_git(
        store,
        &dir,
        &["rev-parse", "--short", "--verify", "-q", "HEAD"],
    )?;
    let head = head.success().then(|| head.stdout.trim().to_string());
    let out = old_git(store, &dir, &["ls-files", "-z"])?;
    if !out.success() {
        anyhow::bail!("git ls-files failed in {from}: {}", out.stderr.trim());
    }
    let paths = out
        .stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    Ok((Source { from, head }, paths))
}

/// One path per line (or NUL-separated); blank lines and `#` comments are ignored.
fn read_list(store: &Store, arg: &str, cwd: &Path) -> Result<(Source, Vec<String>)> {
    let (from, text) = if arg == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .context("reading the path list from stdin")?;
        ("stdin".to_string(), buf)
    } else {
        let p = absolute(store, arg, cwd);
        let text = fs::read(&p).map_err(|e| -> anyhow::Error {
            DwError::usage(format!("cannot read {}: {e}", shown(store, &p))).into()
        })?;
        (shown(store, &p), text)
    };
    let text = String::from_utf8_lossy(&text);
    let sep = if text.contains('\0') { '\0' } else { '\n' };
    let paths = text
        .split(sep)
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    Ok((Source { from, head: None }, paths))
}

/// The listed files that can be considered (existing, inside HOME, not
/// hidden), plus the skipped ones with their reasons. A listed folder stands
/// for its files.
fn sort_out(store: &Store, ctx: &Ctx, listed: &[String]) -> (BTreeSet<String>, Vec<Skipped>) {
    let home = &store.dirs.home;
    let mut files = BTreeSet::new();
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in listed {
        if !seen.insert(raw.as_str()) {
            continue;
        }
        let arg = if raw == "~" || raw.starts_with("~/") || Path::new(raw).is_absolute() {
            raw.clone()
        } else {
            format!("./{raw}")
        };
        let hp = match track::resolve_home_path(store, &arg, home) {
            Ok(hp) => hp,
            Err(e) => {
                skipped.push(Skipped {
                    path: raw.clone(),
                    reason: crate::error::classify(&e).1,
                });
                continue;
            }
        };
        let skip = |reason: String| Skipped {
            path: tilde_rel(&hp.rel),
            reason,
        };
        let Ok(meta) = fs::symlink_metadata(&hp.abs) else {
            skipped.push(skip(format!("~/{} does not exist", hp.rel)));
            continue;
        };
        if let Some(why) = ctx.rules.track_refusal(&hp.abs, &hp.rel, meta.is_dir()) {
            skipped.push(skip(why));
        } else if meta.is_dir() {
            match track::walk_folder(home, &hp.rel, &ctx.rules, None) {
                Ok(w) => files.extend(w.files.into_iter().map(|f| f.path)),
                Err(e) => skipped.push(skip(format!("{e:#}"))),
            }
        } else {
            files.insert(hp.rel);
        }
    }
    (files, skipped)
}

#[derive(Default)]
struct Node {
    direct: Vec<String>,
    subdirs: BTreeSet<String>,
}

fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Folder collapsing. A folder is complete when every non-hidden file in it
/// (recursively) was listed. A complete folder holding listed files directly
/// is tracked as a tracked folder (its subfolders are inside it); a complete
/// folder holding only subfolders gives way to them (the deepest such folder
/// wins); HOME itself never collapses. Files outside complete folders are
/// tracked one by one.
fn group(home: &Path, ctx: &Ctx, listed: &BTreeSet<String>) -> Grouping {
    let mut tree: BTreeMap<String, Node> = BTreeMap::new();
    for f in listed {
        tree.entry(parent(f).to_string())
            .or_default()
            .direct
            .push(f.clone());
        let mut cur = parent(f);
        while !cur.is_empty() {
            let up = parent(cur);
            tree.entry(up.to_string())
                .or_default()
                .subdirs
                .insert(cur.to_string());
            cur = up;
        }
    }
    let mut memo: HashMap<String, bool> = HashMap::new();
    let mut complete = |d: &str| -> bool {
        *memo.entry(d.to_string()).or_insert_with(|| {
            track::any_file(home, d, &ctx.rules, &mut |f| !listed.contains(f))
                .is_ok_and(|unlisted| !unlisted)
        })
    };
    let mut out = Grouping::default();
    // (folder, known complete because an ancestor is)
    let mut stack: Vec<(String, bool)> = vec![(String::new(), false)];
    while let Some((d, known)) = stack.pop() {
        let Some(node) = tree.get(&d) else { continue };
        let whole = !d.is_empty() && (known || complete(&d));
        if whole && !node.direct.is_empty() {
            let below = format!("{d}/");
            out.folders.push(GroupFolder {
                files: listed
                    .range(below.clone()..)
                    .take_while(|f| f.starts_with(&below))
                    .count(),
                path: tilde_rel(&d),
            });
            continue;
        }
        if !whole {
            out.files.extend(node.direct.iter().map(|f| tilde_rel(f)));
        }
        stack.extend(node.subdirs.iter().rev().map(|s| (s.clone(), whole)));
    }
    out.folders.sort_by(|a, b| a.path.cmp(&b.path));
    out.files.sort();
    out
}

/// Read the source and plan the items. Nothing is written.
pub fn plan(store: &Store, from: &From, cwd: &Path, requester: &str) -> Result<ImportPlan> {
    let (source, listed) = match from {
        From::Bare(dir) => read_bare(store, dir, cwd)?,
        From::List(list) => read_list(store, list, cwd)?,
    };
    let ctx = Ctx::load(store)?;
    let (files, mut skipped) = sort_out(store, &ctx, &listed);
    let grouping = group(&store.dirs.home, &ctx, &files);
    let args: Vec<String> = grouping
        .folders
        .iter()
        .map(|f| f.path.clone())
        .chain(grouping.files.iter().cloned())
        .collect();
    let (mut planned, _) =
        track::plan_each(store, Plan::Track, &args, &store.dirs.home, requester)?;
    // A tracked folder leaves flagged files out without a vault (they stay
    // New); name them here too, like the files refused one by one.
    if !crate::vault::exists(store) {
        let inside: Vec<String> = planned
            .items
            .iter()
            .filter_map(|i| match &i.action {
                Action::Track {
                    path, folder: true, ..
                } => Some(format!("{path}/")),
                _ => None,
            })
            .flat_map(|below| {
                files
                    .iter()
                    .filter(move |f| f.starts_with(&below))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        for f in crate::detect::found_in(store, &inside) {
            skipped.push(Skipped {
                path: tilde_rel(&f.rel),
                reason: format!(
                    "possible secret ({}); its folder is tracked without it — run `dw vault init`, then `dw track {}`",
                    f.where_text(),
                    tilde_rel(&f.rel)
                ),
            });
        }
    }
    skipped.append(&mut planned.skipped);
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    planned.skipped = skipped;
    for item in &mut planned.items {
        item.import = Some(source.clone());
    }
    Ok(ImportPlan {
        source,
        listed: listed.len(),
        grouping,
        planned,
    })
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Marker {
    label: String,
    paths: BTreeSet<String>,
}

fn load_marker(store: &Store) -> Option<Marker> {
    let bytes = fs::read(store.dirs.import_file()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Remember the target paths applied import items staged (caller holds the
/// lock). Best effort: the record only shapes a commit message.
pub fn record_applied<'a>(store: &Store, items: impl Iterator<Item = &'a Item>) {
    let mut marker: Option<Marker> = None;
    for item in items {
        let (Some(src), Action::Track { files, .. }) = (&item.import, &item.action) else {
            continue;
        };
        let label = src.label();
        let m = marker.get_or_insert_with(|| match load_marker(store) {
            Some(m) if m.label == label => m,
            _ => Marker {
                label,
                paths: BTreeSet::new(),
            },
        });
        m.paths.extend(files.iter().map(|f| f.path.clone()));
    }
    if let Some(m) = marker
        && let Ok(bytes) = serde_json::to_vec_pretty(&m)
    {
        let _ = crate::fsutil::write_atomic(&store.dirs.import_file(), bytes);
    }
}

/// The import label when one of the staged paths (`~/…`, as shown) came from
/// an applied import item since the last commit.
pub fn prefix<'a>(store: &Store, staged: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let m = load_marker(store)?;
    let ours: BTreeSet<String> = m.paths.iter().map(|p| tilde_rel(p)).collect();
    staged
        .into_iter()
        .any(|p| ours.contains(p))
        .then_some(m.label)
}

/// A commit was made: staged changes no longer come from the import.
pub fn committed(store: &Store) {
    let _ = fs::remove_file(store.dirs.import_file());
}
