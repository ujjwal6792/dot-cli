//! The change model (spec §5, ADR 0003): classifying tracked files into Local,
//! New, Staged, Incoming and Conflict, staging, unstaging, committing and
//! diffing. HOME is the working tree; the repository's own working tree always
//! equals its index.
//!
//! Classification uses `chezmoi status` (column 1: last written vs HOME, column
//! 2: HOME vs target), chezmoi's last-written state and git's staged paths.
//! `chezmoi add` records the added content as last written, so a staged and
//! committed file shows no change; unstaging puts the last-written record back.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{DwError, Kind};
use crate::exec;
use crate::pending::{Action, FileChange, Item, Preview};
use crate::snapshot;
use crate::source::{self, decode_source_path};
use crate::store::Store;
use crate::track::{self, Ctx, git_literal, refuse};

const CHUNK: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Local,
    New,
    Staged,
    Incoming,
    Conflict,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Local => "local",
            State::New => "new",
            State::Staged => "staged",
            State::Incoming => "incoming",
            State::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Relative to HOME, or relative to the repository for `kind == "repo"`.
    pub path: String,
    pub state: State,
    /// `modified`, `deleted` (gone from HOME), `new`, `removed` (staged
    /// untrack) or `repo` (a staged repository file that is not a HOME file).
    pub kind: &'static str,
}

struct Target {
    link: bool,
    /// File contents, or the link target for symlinks.
    text: String,
    perm: u64,
}

impl Target {
    fn sha(&self) -> String {
        sha256(self.text.as_bytes())
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Files and symlinks that apply to this machine, with their rendered content.
fn targets(store: &Store) -> Result<BTreeMap<String, Target>> {
    let mut args: Vec<String> = ["dump", "--format", "json", "--include=files,symlinks"]
        .map(String::from)
        .into();
    match crate::variants::render_scope(store)? {
        Some(paths) if paths.is_empty() => return Ok(BTreeMap::new()),
        Some(paths) => args.extend(paths),
        None => {}
    }
    let out = store.chezmoi(&args)?;
    let v: Value = serde_json::from_str(&out.stdout).context("reading chezmoi dump")?;
    let mut map = BTreeMap::new();
    for (rel, e) in v.as_object().into_iter().flatten() {
        let link = e["type"] == "symlink";
        let text = e[if link { "linkname" } else { "contents" }]
            .as_str()
            .unwrap_or_default()
            .to_string();
        map.insert(
            rel.clone(),
            Target {
                link,
                text,
                perm: e["perm"].as_u64().unwrap_or(0o644),
            },
        );
    }
    Ok(map)
}

/// Last-written records: absolute path to (type, content hash).
fn last_written(store: &Store) -> Result<BTreeMap<String, (String, String)>> {
    let out = store.chezmoi(&["state", "dump", "--format", "json"])?;
    let v: Value = serde_json::from_str(&out.stdout).context("reading chezmoi state")?;
    Ok(v["entryState"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, e)| {
            (
                k.clone(),
                (
                    e["type"].as_str().unwrap_or_default().to_string(),
                    e["contentsSHA256"].as_str().unwrap_or_default().to_string(),
                ),
            )
        })
        .collect())
}

/// `chezmoi status` columns per target path.
fn status_columns(store: &Store) -> Result<BTreeMap<String, (char, char)>> {
    let mut args: Vec<String> = ["status", "--include=files,symlinks"]
        .map(String::from)
        .into();
    match crate::variants::render_scope(store)? {
        Some(paths) if paths.is_empty() => return Ok(BTreeMap::new()),
        Some(paths) => args.extend(paths),
        None => {}
    }
    let out = store.chezmoi(&args)?;
    Ok(out
        .stdout
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let mut c = l.chars();
            (l[3..].to_string(), (c.next().unwrap(), c.next().unwrap()))
        })
        .collect())
}

/// Paths changed between HEAD and the index: `(status letter, repo path)`.
fn staged_paths(store: &Store) -> Result<Vec<(char, String)>> {
    let o = git_literal(
        store,
        &[
            "diff".as_ref(),
            "--cached".as_ref(),
            "--name-status".as_ref(),
            "--no-renames".as_ref(),
            "-z".as_ref(),
        ],
    )?;
    let mut parts = o.stdout.split('\0').filter(|s| !s.is_empty());
    let mut out = Vec::new();
    while let (Some(s), Some(p)) = (parts.next(), parts.next()) {
        out.push((s.chars().next().unwrap_or('M'), p.to_string()));
    }
    Ok(out)
}

pub struct Scan {
    pub changes: Vec<Change>,
    /// Files whose variants clash on this machine (path, reason): not classified.
    pub clashes: BTreeMap<String, String>,
    /// Files that need the vault while it is locked (not classified).
    pub locked: std::collections::BTreeSet<String>,
    targets: BTreeMap<String, Target>,
}

/// Classify every change on this machine.
pub fn scan(store: &Store, ctx: &Ctx) -> Result<Scan> {
    let home = &store.dirs.home;
    let repo = store.repo();
    let targets = targets(store)?;
    let columns = status_columns(store)?;
    let last = last_written(store)?;
    let clashes = crate::variants::clashes(store)?;
    let locked = crate::values::locked_files(store)?;
    let vars = crate::variants::Variants::read(&repo)?;
    let mut changes = Vec::new();

    let mut staged: BTreeMap<String, &'static str> = BTreeMap::new();
    for (letter, path) in staged_paths(store)? {
        match repo_target(&repo, &vars, &path) {
            Some(target) => {
                let present = fs::symlink_metadata(repo.join(&path)).is_ok();
                if present
                    && !targets.contains_key(&target)
                    && !clashes.contains_key(&target)
                    && !locked.contains(&target)
                {
                    continue; // assigned to other machines
                }
                let kind = match letter {
                    'A' => "new",
                    'D' => "removed",
                    _ => "modified",
                };
                staged
                    .entry(target)
                    .and_modify(|k| *k = "modified")
                    .or_insert(kind);
            }
            None => changes.push(Change {
                path,
                state: State::Staged,
                kind: "repo",
            }),
        }
    }
    for (path, kind) in &staged {
        changes.push(Change {
            path: path.clone(),
            state: State::Staged,
            kind,
        });
    }

    for (rel, t) in &targets {
        let (c1, c2) = columns.get(rel).copied().unwrap_or((' ', ' '));
        if c2 == ' ' {
            continue;
        }
        let missing = fs::symlink_metadata(home.join(rel)).is_err();
        let (state, kind) = if staged.contains_key(rel) {
            (State::Local, "modified")
        } else {
            let kind = if missing { "new" } else { "modified" };
            match last.get(&home.join(rel).to_string_lossy().into_owned()) {
                _ if c1 == ' ' => (State::Incoming, kind),
                None if missing => (State::Incoming, "new"),
                None => (State::Conflict, "modified"),
                Some((ty, sha)) => {
                    let same = *ty == if t.link { "symlink" } else { "file" } && *sha == t.sha();
                    match (same, missing) {
                        (true, true) => (State::Local, "deleted"),
                        (true, false) => (State::Local, "modified"),
                        _ => (State::Conflict, "modified"),
                    }
                }
            }
        };
        changes.push(Change {
            path: rel.clone(),
            state,
            kind,
        });
    }

    for path in track::new_files(home, &ctx.tracked, &ctx.rules, None)? {
        changes.push(Change {
            path,
            state: State::New,
            kind: "new",
        });
    }
    changes.sort_by(|a, b| (a.state, &a.path).cmp(&(b.state, &b.path)));
    Ok(Scan {
        changes,
        clashes,
        locked,
        targets,
    })
}

/// The tracked path a repository file stands for: a source file, or a variant
/// file of a tracked path.
fn repo_target(repo: &Path, vars: &crate::variants::Variants, path: &str) -> Option<String> {
    decode_source_path(repo, path)
        .map(|(t, _)| t)
        .or_else(|| vars.target_of(path))
}

pub fn classify(store: &Store) -> Result<Vec<Change>> {
    Ok(scan(store, &Ctx::load(store)?)?.changes)
}

fn covers(selector: &str, path: &str) -> bool {
    path == selector
        || path
            .strip_prefix(selector)
            .is_some_and(|r| r.starts_with('/'))
}

fn selected(sel: Option<&[String]>, path: &str) -> bool {
    sel.is_none_or(|s| s.iter().any(|x| covers(x, path)))
}

fn tilde(c: &Change) -> String {
    if c.kind == "repo" {
        format!("repo:{}", c.path)
    } else {
        track::tilde(&c.path)
    }
}

fn no_match(sel: &[String], matched: impl Fn(&str) -> bool, verb: &str) -> Result<()> {
    let missing: Vec<_> = sel.iter().filter(|s| !matched(s)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(refuse(format!(
        "nothing to {verb} at {}",
        missing
            .iter()
            .map(|s| track::tilde(s))
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct Done {
    pub paths: Vec<String>,
    pub skipped: Vec<Skipped>,
    pub dry_run: bool,
    /// Variables that staging updates, and wiring it proposes.
    pub notes: Vec<String>,
}

/// Stage HOME content of the selected changes (`None` selects everything that
/// can be staged). Explicitly named changes that cannot be staged are refused;
/// with `None`, or inside a named folder, they are skipped and reported.
pub fn stage(store: &Store, sel: Option<&[String]>, dry_run: bool) -> Result<Done> {
    let lock = if dry_run { None } else { Some(store.lock()?) };
    let ctx = Ctx::load(store)?;
    let scan = scan(store, &ctx)?;
    let mut files = Vec::new();
    let mut deleted = Vec::new();
    let mut skipped = Vec::new();
    let mut refusals = Vec::new();
    let vars = crate::variants::Variants::read(&store.repo())?;
    let mine = crate::variants::this_targets(store)?;
    let resolver = crate::values::Resolver::load(store)?;
    // (HOME path, variant file) of variant files: their content goes to the variant received here.
    let mut variant_files: Vec<(String, String)> = Vec::new();
    // Files with references: their template is rebuilt from HOME (spec §7).
    let mut ref_files: Vec<crate::values::RefStage> = Vec::new();
    for s in sel.unwrap_or_default() {
        if let Some(why) = scan.clashes.get(s) {
            return Err(refuse(format!("~/{s}: {why}")));
        }
        if scan.locked.iter().any(|l| covers(s, l)) {
            return Err(DwError::new(
                Kind::VaultLocked,
                format!(
                    "~/{s} needs the vault, which is locked; run `dw vault unlock` in a terminal"
                ),
            )
            .into());
        }
    }
    let matched_any = |p: &str| {
        scan.changes
            .iter()
            .any(|c| c.kind != "repo" && covers(p, &c.path))
    };
    if let Some(s) = sel {
        no_match(s, matched_any, "stage")?;
    }
    for c in &scan.changes {
        if c.kind == "repo" || !selected(sel, &c.path) {
            continue;
        }
        let explicit = sel.is_some_and(|s| s.contains(&c.path));
        if c.state == State::Local && c.kind != "deleted" && vars.0.contains_key(&c.path) {
            let found = variant_for_stage(store, &vars, mine.as_deref(), &c.path).and_then(|f| {
                match crate::values::stage_plan(
                    store,
                    &resolver,
                    &c.path,
                    &crate::variants::repo_path(&f),
                    true,
                ) {
                    Ok(Some(p)) if p.unchanged => Err(MODE_ONLY.to_string()),
                    Ok(Some(p)) => {
                        ref_files.push(p);
                        Ok(None)
                    }
                    Ok(None) => Ok(Some(f)),
                    Err(reason) => Err(reason),
                }
            });
            match found {
                Ok(Some(f)) => variant_files.push((c.path.clone(), f)),
                Ok(None) => {}
                Err(reason) if explicit => {
                    refusals.push((Kind::Error, format!("~/{}: {reason}", c.path)))
                }
                Err(reason) => skipped.push(Skipped {
                    path: track::tilde(&c.path),
                    reason,
                }),
            }
            continue;
        }
        let problem: Option<(Kind, String)> = match c.state {
            State::Staged => continue,
            State::Incoming => Some((
                Kind::Error,
                "an incoming change would be overwritten; apply it first".into(),
            )),
            State::Conflict => Some((
                Kind::Conflict,
                "changed here and by an incoming change; resolve the conflict first".into(),
            )),
            _ if c.kind == "deleted" => {
                deleted.push(c.path.clone());
                continue;
            }
            _ if ctx
                .tracked
                .files
                .get(&c.path)
                .is_some_and(|e| e.source.ends_with(".tmpl")) =>
            {
                let src = ctx.tracked.files[&c.path].source.clone();
                match crate::values::stage_plan(store, &resolver, &c.path, &src, false) {
                    Ok(Some(p)) if !p.unchanged => {
                        ref_files.push(p);
                        continue;
                    }
                    Ok(_) => Some((Kind::Error, MODE_ONLY.to_string())),
                    Err(reason) => Some((Kind::Error, reason)),
                }
            }
            _ => None,
        };
        match problem {
            None => files.push(c.path.clone()),
            Some((kind, reason)) if explicit => {
                refusals.push((kind, format!("~/{}: {reason}", c.path)))
            }
            Some((_, reason)) => skipped.push(Skipped {
                path: track::tilde(&c.path),
                reason,
            }),
        }
    }
    if let Some((kind, _)) = refusals.first() {
        let msg = refusals
            .iter()
            .map(|(_, m)| m.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(DwError::new(*kind, msg).into());
    }
    let mut notes: Vec<String> = Vec::new();
    let mut new_values: BTreeMap<&str, &str> = BTreeMap::new();
    for ch in ref_files.iter().flat_map(|p| &p.changes) {
        if let Some(other) = new_values.insert(&ch.name, &ch.new)
            && other != ch.new
        {
            return Err(refuse(if resolver.is_secret(&ch.name) {
                format!(
                    "secret {} would get two values; stage one file at a time",
                    ch.name
                )
            } else {
                format!(
                    "variable {} would get two values ({other:?} and {:?}); stage one file at a time",
                    ch.name, ch.new
                )
            }));
        }
        let line = crate::values::describe(&resolver, ch);
        if !notes.contains(&line) {
            notes.push(line);
        }
    }
    let nothing =
        files.is_empty() && deleted.is_empty() && variant_files.is_empty() && ref_files.is_empty();
    if nothing && skipped.is_empty() {
        bail!("nothing to stage");
    }
    if !(dry_run || nothing) {
        track::ensure_repo_clean(store)?;
        let tree = track::index_tree(store)?;
        let backup = track::state_backup(store)?;
        let mut results = Vec::new();
        let mut run = || -> Result<()> {
            for chunk in files.chunks(CHUNK) {
                let mut args: Vec<std::ffi::OsString> = vec!["add".into()];
                args.extend(chunk.iter().map(|f| store.dirs.home.join(f).into()));
                store.chezmoi_locked(&args)?;
            }
            for (rel, file) in &variant_files {
                let dest = store.dirs.home.join(rel);
                let bytes = fs::read(&dest).with_context(|| format!("reading ~/{rel}"))?;
                let to = store.repo().join(crate::variants::repo_path(file));
                fs::write(&to, bytes).with_context(|| format!("writing {}", to.display()))?;
            }
            for p in &ref_files {
                let to = store.repo().join(&p.dest);
                fs::write(&to, &p.bytes).with_context(|| format!("writing {}", to.display()))?;
            }
            let all: Vec<_> = ref_files.iter().flat_map(|p| p.changes.clone()).collect();
            crate::values::apply_changes(store, &all)?;
            if !(variant_files.is_empty() && ref_files.is_empty()) {
                // The rendered file now equals HOME, which is what was last written.
                let rels: BTreeSet<&str> = variant_files
                    .iter()
                    .map(|(r, _)| r.as_str())
                    .chain(ref_files.iter().map(|p| p.rel.as_str()))
                    .collect();
                restore_last_written(store, &rels)?;
            }
            let changed: Vec<String> = track::unstaged(store)?.into_iter().collect();
            track::stage(store, &changed)?;
            if !track::unstaged(store)?.is_empty() {
                bail!("some changes could not be staged");
            }
            // A file deleted in HOME is staged by untracking it.
            for rel in &deleted {
                let hp = track::HomePath {
                    abs: store.dirs.home.join(rel),
                    rel: rel.clone(),
                };
                let item = track::plan_untrack(store, &Ctx::load(store)?, &hp, "human")?;
                let lock = lock.as_ref().expect("held outside dry runs");
                let done = track::apply_items(store, lock, &[item])?;
                if let Some(r) = done.iter().find(|r| r.status != track::Outcome::Applied) {
                    bail!(
                        "~/{rel}: {}",
                        r.message.as_deref().unwrap_or("could not be untracked")
                    );
                }
                results.extend(done);
            }
            Ok(())
        };
        if let Err(e) = run() {
            track::rollback(store, &tree)?;
            track::state_restore(store, backup)?;
            return Err(e.context("staging failed; the repository was left as before"));
        }
        let staged: Vec<String> = files
            .iter()
            .cloned()
            .chain(variant_files.iter().map(|(r, _)| r.clone()))
            .chain(ref_files.iter().map(|p| p.rel.clone()))
            .collect();
        if let Some(l) = lock.as_ref() {
            // Detection only proposes; it never fails a stage that succeeded.
            notes.extend(crate::values::propose_wires(store, l, &staged).unwrap_or_default());
        }
        // chezmoi's records of untracked files go once the mutation lock is free.
        drop(lock);
        track::cleanup_state(store, &results);
    }
    Ok(Done {
        paths: files
            .iter()
            .chain(variant_files.iter().map(|(rel, _)| rel))
            .chain(ref_files.iter().map(|p| &p.rel))
            .map(|f| track::tilde(f))
            .chain(
                deleted
                    .iter()
                    .map(|f| format!("{} (deleted; untracked)", track::tilde(f))),
            )
            .collect(),
        skipped,
        dry_run,
        notes,
    })
}

const MODE_ONLY: &str = "only its mode differs; mode changes of variant files are not supported";

/// The variant file that receives the HOME content of `rel` when staging on this
/// machine: the variant this machine currently receives, else the default.
fn variant_for_stage(
    store: &Store,
    vars: &crate::variants::Variants,
    mine: Option<&[String]>,
    rel: &str,
) -> Result<String, String> {
    use crate::variants::{Pick, default_file, pick, repo_path};
    let meta = fs::symlink_metadata(store.dirs.home.join(rel)).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("is not a regular file; variants hold file content".into());
    }
    let mine = mine.ok_or("this machine is not in the registry")?;
    let file = match pick(vars.get(rel), mine) {
        Pick::One(v) => v.file.clone(),
        Pick::Default => default_file(rel),
        Pick::Clash(t) => return Err(format!("variants clash: {}", t.join(", "))),
    };
    let stored = fs::read(store.repo().join(repo_path(&file))).ok();
    if stored.is_some() && stored == fs::read(store.dirs.home.join(rel)).ok() {
        Err(MODE_ONLY.into())
    } else if stored.is_some() {
        Ok(file)
    } else {
        Err(format!("its variant file {} is missing", repo_path(&file)))
    }
}

fn prune_empty_dirs(repo: &Path, file: &Path) {
    let root = repo.join(source::source_root(repo));
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(&root) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Unstage the selected changes: restore their repository entries from HEAD.
/// `None` selects everything staged, including repository files that are not
/// HOME files (such as a staged untrack's data changes).
pub fn unstage(store: &Store, sel: Option<&[String]>, dry_run: bool) -> Result<Done> {
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    let repo = store.repo();
    let ctx = Ctx::load(store)?;
    let vars = crate::variants::Variants::read(&repo)?;
    let changes: Vec<Change> = scan(store, &ctx)?
        .changes
        .into_iter()
        .filter(|c| c.state == State::Staged && (c.kind != "repo" || sel.is_none()))
        .filter(|c| selected(sel, &c.path))
        .collect();
    if let Some(s) = sel {
        no_match(s, |p| changes.iter().any(|c| covers(p, &c.path)), "unstage")?;
    }
    if changes.is_empty() {
        bail!("nothing is staged");
    }
    let selected_targets: BTreeSet<&str> = changes
        .iter()
        .filter(|c| c.kind != "repo")
        .map(|c| c.path.as_str())
        .collect();
    let repo_paths: BTreeSet<&str> = changes
        .iter()
        .filter(|c| c.kind == "repo")
        .map(|c| c.path.as_str())
        .collect();
    let sources: Vec<String> = staged_paths(store)?
        .into_iter()
        .map(|(_, p)| p)
        .filter(|p| match repo_target(&repo, &vars, p) {
            Some(t) => selected_targets.contains(t.as_str()),
            None => repo_paths.contains(p.as_str()),
        })
        .collect();
    let done = Done {
        paths: changes.iter().map(tilde).collect(),
        skipped: Vec::new(),
        dry_run,
        notes: Vec::new(),
    };
    if dry_run {
        return Ok(done);
    }
    track::ensure_repo_clean(store)?;
    let tree = track::index_tree(store)?;
    let backup = track::state_backup(store)?;
    let run = || -> Result<()> {
        let in_head = |p: &str| {
            git_literal(
                store,
                &[
                    "cat-file".as_ref(),
                    "-e".as_ref(),
                    format!("HEAD:{p}").as_ref(),
                ],
            )
            .is_ok()
        };
        let (present, gone): (Vec<&String>, Vec<&String>) =
            sources.iter().partition(|p| in_head(p));
        fn paths<'a>(v: &[&'a String]) -> Vec<&'a OsStr> {
            v.iter().map(|p| OsStr::new(p.as_str())).collect()
        }
        for set in [&present, &gone] {
            if set.is_empty() {
                continue;
            }
            let mut args: Vec<&OsStr> = vec![
                "reset".as_ref(),
                "-q".as_ref(),
                "HEAD".as_ref(),
                "--".as_ref(),
            ];
            args.extend(paths(set));
            git_literal(store, &args)?;
        }
        if !present.is_empty() {
            let mut args: Vec<&OsStr> = vec!["checkout".as_ref(), "--".as_ref()];
            args.extend(paths(&present));
            git_literal(store, &args)?;
        }
        for p in gone {
            let file = repo.join(p);
            fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
            prune_empty_dirs(&repo, &file);
        }
        restore_last_written(store, &selected_targets)
    };
    if let Err(e) = run() {
        track::rollback(store, &tree)?;
        track::state_restore(store, backup)?;
        return Err(e.context("unstaging failed; the repository was left as before"));
    }
    Ok(done)
}

/// Put chezmoi's last-written record of each target back to the committed
/// version (or drop it for files that are no longer tracked), so the file shows
/// as Local rather than Incoming.
pub(crate) fn restore_last_written(store: &Store, targets: &BTreeSet<&str>) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let committed = self::targets(store)?;
    for rel in targets {
        let key = store.dirs.home.join(rel).to_string_lossy().into_owned();
        match committed.get(*rel) {
            Some(t) => {
                let value = if t.link {
                    serde_json::json!({"type": "symlink", "contentsSHA256": t.sha()})
                } else {
                    serde_json::json!({"type": "file", "mode": t.perm, "contentsSHA256": t.sha()})
                };
                store.chezmoi_locked(&[
                    "state".to_string(),
                    "set".into(),
                    "--bucket=entryState".into(),
                    format!("--key={key}"),
                    format!("--value={value}"),
                ])?;
            }
            None => {
                store.chezmoi_locked(&[
                    "state".to_string(),
                    "delete".into(),
                    "--bucket=entryState".into(),
                    format!("--key={key}"),
                ])?;
            }
        }
    }
    Ok(())
}

// ---- discard -----------------------------------------------------------

/// Snapshot `files` and write their committed version back to HOME. Returns
/// the snapshot id when something was preserved.
pub fn discard_files(store: &Store, files: &[String], actor: &str) -> Result<Option<String>> {
    let snap = snapshot::take(store, files, "discard", actor)?.map(|s| s.id);
    for chunk in files.chunks(CHUNK) {
        let mut args: Vec<std::ffi::OsString> =
            vec!["apply".into(), "--force".into(), "--parent-dirs".into()];
        args.extend(chunk.iter().map(|f| store.dirs.home.join(f).into()));
        store.chezmoi_locked(&args)?;
    }
    Ok(snap)
}

/// Plan the discard of the local changes at `rel` (a file or folder) as a
/// pending item. Only Local changes can be discarded.
pub fn plan_discard(store: &Store, rel: &str, requester: &str) -> Result<Item> {
    let ctx = Ctx::load(store)?;
    let scan = scan(store, &ctx)?;
    let mut files = Vec::new();
    let mut problems = Vec::new();
    for c in scan
        .changes
        .iter()
        .filter(|c| c.kind != "repo" && covers(rel, &c.path))
    {
        let exact = c.path == rel;
        match (c.state, c.kind) {
            (State::Local, _) => files.push(c.path.clone()),
            (State::Staged, _) => {}
            (State::New, _) if exact => problems.push(format!(
                "~/{} is not tracked yet; there is no committed version to restore",
                c.path
            )),
            (State::Incoming | State::Conflict, _) if exact => problems.push(format!(
                "~/{} has an incoming change; resolving it is not supported yet",
                c.path
            )),
            _ => {}
        }
    }
    if let Some(p) = problems.first() {
        return Err(refuse(p.clone()));
    }
    files.sort();
    files.dedup();
    if files.is_empty() {
        return Err(refuse(format!(
            "no local changes to discard at {}",
            track::tilde(rel)
        )));
    }
    let preview = Preview {
        applies_to: None,
        files: files
            .iter()
            .map(|f| FileChange {
                action: "overwrite".into(),
                target: f.clone(),
                source: String::new(),
                kind: "file".into(),
            })
            .collect(),
        data: Vec::new(),
        rules: Vec::new(),
        notes: vec!["a snapshot of the current files is taken first (`dw snapshot list`)".into()],
        ..Preview::default()
    };
    let summary = format!(
        "Discard local changes to {} (requested by {requester})",
        if files.len() == 1 {
            track::tilde(&files[0])
        } else {
            format!("{} files under {}", files.len(), track::tilde(rel))
        }
    );
    let action = Action::Discard {
        path: rel.to_string(),
        files,
    };
    track::new_item(store, requester, action, summary, preview, &ctx)
}

// ---- apply incoming ------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Applied {
    pub dry_run: bool,
    pub applied: Vec<String>,
    /// Files changed on both sides: apply is blocked until they are resolved.
    pub conflicts: Vec<String>,
    /// Files chezmoi could not write (the others were written regardless).
    pub failed: Vec<String>,
    /// Files that need the vault, skipped because it is locked.
    pub vault_locked: Vec<String>,
    pub snapshot: Option<String>,
    pub error: Option<String>,
}

/// Write Incoming files to HOME, snapshotting them first. `paths` selects files
/// or folders (`None`: all). Only Incoming files are ever written: chezmoi is
/// given exactly those targets and no `--force`, so a file that changed in HOME
/// after the scan is refused by chezmoi instead of overwritten. A Conflict named
/// exactly is refused (exit 5); with `--all` or a folder it is skipped and listed.
pub fn apply_incoming(store: &Store, paths: Option<&[String]>, dry_run: bool) -> Result<Applied> {
    let _lock = (!dry_run).then(|| store.lock()).transpose()?;
    let ctx = Ctx::load(store)?;
    let found = scan(store, &ctx)?;
    let pick = |state: State| -> Vec<String> {
        let mut v: Vec<String> = found
            .changes
            .iter()
            .filter(|c| c.state == state && c.kind != "repo" && selected(paths, &c.path))
            .map(|c| c.path.clone())
            .collect();
        v.sort();
        v
    };
    let files = pick(State::Incoming);
    let conflicts = pick(State::Conflict);
    let vault_locked: Vec<String> = found
        .locked
        .iter()
        .filter(|l| selected(paths, l))
        .cloned()
        .collect();
    for p in paths.unwrap_or_default() {
        if vault_locked.iter().any(|l| covers(p, l)) {
            return Err(DwError::new(
                Kind::VaultLocked,
                format!(
                    "~/{p} needs the vault, which is locked; run `dw vault unlock` in a terminal"
                ),
            )
            .into());
        }
        if conflicts.contains(p) {
            return Err(DwError::new(
                Kind::Conflict,
                format!(
                    "~/{p} changed here and in the repository; applying would lose a side. \
                     Resolving conflicts arrives later"
                ),
            )
            .into());
        }
        if !files.iter().any(|f| covers(p, f)) && !conflicts.iter().any(|f| covers(p, f)) {
            return Err(refuse(format!("nothing incoming at ~/{p}")));
        }
    }
    let mut out = Applied {
        dry_run,
        applied: files.clone(),
        conflicts,
        failed: Vec::new(),
        vault_locked,
        snapshot: None,
        error: None,
    };
    if dry_run || files.is_empty() {
        return Ok(out);
    }
    let actor = agent(store).unwrap_or_else(|| "human".into());
    out.snapshot = snapshot::take(store, &files, "apply", &actor)?.map(|s| s.id);
    let mut errors = Vec::new();
    for chunk in files.chunks(CHUNK) {
        let mut args: Vec<std::ffi::OsString> = vec![
            "apply".into(),
            "--keep-going".into(),
            "--parent-dirs".into(),
        ];
        args.extend(chunk.iter().map(|f| store.dirs.home.join(f).into()));
        if let Err(e) = store.chezmoi_locked(&args) {
            errors.push(format!("{e:#}"));
        }
    }
    let still: BTreeSet<String> = scan(store, &Ctx::load(store)?)?
        .changes
        .into_iter()
        .filter(|c| c.state == State::Incoming)
        .map(|c| c.path)
        .collect();
    out.failed = files
        .iter()
        .filter(|f| still.contains(*f))
        .cloned()
        .collect();
    out.applied.retain(|f| !still.contains(f));
    if !out.failed.is_empty() {
        out.error = Some(errors.join("; "));
    }
    Ok(out)
}

// ---- commit ------------------------------------------------------------

pub struct CommitOpts {
    pub messages: Vec<String>,
    pub amend: bool,
    pub dry_run: bool,
    /// Whether an editor may be opened (stdin and stdout are terminals).
    pub interactive: bool,
}

#[derive(Debug, Serialize)]
pub struct Committed {
    pub commit: Option<String>,
    pub subject: String,
    pub files: usize,
    pub amended: bool,
    pub agent: Option<String>,
    pub dry_run: bool,
}

pub fn agent(store: &Store) -> Option<String> {
    store
        .env()
        .get(OsStr::new("DOTWARDEN_AGENT"))
        .and_then(|v| v.to_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn edit_message(store: &Store, initial: &str) -> Result<String> {
    let file = store.dirs.state_dir().join("COMMIT_MESSAGE");
    let body = format!(
        "{initial}\n# Write the commit message. Lines starting with '#' are ignored;\n# an empty message aborts the commit.\n"
    );
    crate::fsutil::write_atomic(&file, body)?;
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|k| store.env().get(OsStr::new(k)).filter(|v| !v.is_empty()))
        .cloned()
        .unwrap_or_else(|| "vi".into());
    let mut cmd = std::process::Command::new("sh");
    let mut script = editor;
    script.push(" \"$1\"");
    cmd.arg("-c").arg(script).arg("sh").arg(&file);
    cmd.env_clear().envs(store.env());
    let status = cmd.status().context("starting the editor")?;
    let text = fs::read_to_string(&file).unwrap_or_default();
    let _ = fs::remove_file(&file);
    if !status.success() {
        bail!("the editor exited with {status}; nothing was committed");
    }
    Ok(text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}

/// Refuse amending a commit that is already on a remote-tracking branch.
pub fn ensure_amendable(store: &Store) -> Result<()> {
    let head = git_literal(
        store,
        &["rev-parse".as_ref(), "--verify".as_ref(), "HEAD".as_ref()],
    )
    .context("there is no commit to amend")?;
    let remote = git_literal(
        store,
        &[
            "branch".as_ref(),
            "-r".as_ref(),
            "--contains".as_ref(),
            OsStr::new(head.stdout.trim()),
        ],
    )?;
    if !remote.stdout.trim().is_empty() {
        return Err(refuse(format!(
            "HEAD is already on {}; amending it would rewrite shared history",
            remote.stdout.lines().next().unwrap_or("a remote").trim()
        )));
    }
    Ok(())
}

/// The full message of the last commit.
pub fn last_message(store: &Store) -> Result<String> {
    Ok(git_literal(
        store,
        &["log".as_ref(), "-1".as_ref(), "--format=%B".as_ref()],
    )?
    .stdout
    .trim()
    .to_string())
}

pub fn commit(store: &Store, opts: &CommitOpts) -> Result<Committed> {
    let _lock = if opts.dry_run {
        None
    } else {
        Some(store.lock()?)
    };
    crate::repo::require_git_identity(store)?;
    let staged = staged_paths(store)?;
    if staged.is_empty() && !opts.amend {
        bail!("nothing is staged; stage changes with `dw stage` first");
    }
    if opts.amend {
        ensure_amendable(store)?;
    }
    crate::detect::commit_guard(store)?;
    if opts.messages.iter().any(|m| m.trim().is_empty()) {
        return Err(DwError::usage("the commit message is empty").into());
    }
    let mut messages = opts.messages.clone();
    let mut reuse = false;
    if messages.is_empty() {
        if opts.interactive {
            let initial = if opts.amend {
                git_literal(
                    store,
                    &["log".as_ref(), "-1".as_ref(), "--format=%B".as_ref()],
                )?
                .stdout
                .trim()
                .to_string()
            } else {
                String::new()
            };
            let text = if opts.dry_run {
                initial
            } else {
                edit_message(store, &initial)?
            };
            if text.is_empty() {
                return Err(DwError::usage("empty commit message; nothing was committed").into());
            }
            messages.push(text);
        } else if opts.amend {
            reuse = true;
        } else {
            return Err(DwError::usage(
                "a commit message is required: pass -m (repeatable), or run in a terminal to use $EDITOR",
            )
            .into());
        }
    }
    let agent = agent(store);
    let subject = messages
        .first()
        .and_then(|m| m.lines().next())
        .unwrap_or("(previous message)")
        .to_string();
    let mut done = Committed {
        commit: None,
        subject,
        files: staged.len(),
        amended: opts.amend,
        agent: agent.clone(),
        dry_run: opts.dry_run,
    };
    if opts.dry_run {
        return Ok(done);
    }
    let mut args: Vec<String> = vec!["commit".into(), "--quiet".into()];
    if opts.amend {
        args.push("--amend".into());
    }
    if reuse {
        args.push("--no-edit".into());
    }
    for m in &messages {
        args.push("-m".into());
        args.push(m.clone());
    }
    if let Some(a) = &agent {
        args.push("--trailer".into());
        args.push(format!("Agent: {a}"));
    }
    store.git(&args)?;
    done.commit = Some(
        store
            .git(&["rev-parse", "--short", "HEAD"])?
            .stdout
            .trim()
            .to_string(),
    );
    Ok(done)
}

// ---- diff --------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub state: State,
    pub diff: String,
}

fn unified(label: &str, old: Option<&str>, new: Option<&str>) -> String {
    let mut out = format!("diff --git a/{label} b/{label}\n");
    if old.is_none() {
        out.push_str("new file mode 100644\n");
    }
    if new.is_none() {
        out.push_str("deleted file mode 100644\n");
    }
    if old.is_some_and(|s| s.contains('\0')) || new.is_some_and(|s| s.contains('\0')) {
        out.push_str(&format!("Binary files a/{label} and b/{label} differ\n"));
        return out;
    }
    let a = old.map_or("/dev/null".to_string(), |_| format!("a/{label}"));
    let b = new.map_or("/dev/null".to_string(), |_| format!("b/{label}"));
    let text = similar::TextDiff::from_lines(old.unwrap_or(""), new.unwrap_or(""))
        .unified_diff()
        .context_radius(3)
        .header(&a, &b)
        .to_string();
    out.push_str(&text);
    out
}

fn home_text(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return Some(fs::read_link(path).ok()?.to_string_lossy().into_owned());
    }
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Plain unified diffs of the Local, New and Staged changes under `sel`.
pub fn diff(store: &Store, sel: Option<&[String]>) -> Result<Vec<FileDiff>> {
    let repo = store.repo();
    let ctx = Ctx::load(store)?;
    let scan = scan(store, &ctx)?;
    let staged = staged_paths(store)?;
    let vars = crate::variants::Variants::read(&repo)?;
    let vchanges = crate::values::value_changes(store, &ctx, &scan.changes)?;
    let resolver = crate::values::Resolver::load(store)?;
    let known = crate::detect::known_pairs(store);
    let mut out = Vec::new();
    for c in &scan.changes {
        let wanted = if c.kind == "repo" {
            sel.is_none()
        } else {
            selected(sel, &c.path)
        };
        if !wanted {
            continue;
        }
        let diff = match c.state {
            State::Local | State::New => {
                let old = scan.targets.get(&c.path).map(|t| t.text.clone());
                let new = home_text(&store.dirs.home.join(&c.path));
                let (old, new) = match new.as_deref().and_then(|h| {
                    crate::values::masked_diff(store, &ctx, &resolver, &vars, &c.path, h)
                }) {
                    Some((o, n)) => (Some(o), Some(n)),
                    None => (old, new),
                };
                unified(&c.path, old.as_deref(), new.as_deref())
            }
            State::Staged if c.kind == "repo" => {
                git_literal(
                    store,
                    &[
                        "diff".as_ref(),
                        "--cached".as_ref(),
                        "--no-color".as_ref(),
                        "--".as_ref(),
                        OsStr::new(&c.path),
                    ],
                )?
                .stdout
            }
            State::Staged => {
                let mut parts = Vec::new();
                for (letter, p) in &staged {
                    if repo_target(&repo, &vars, p).is_none_or(|t| t != c.path) {
                        continue;
                    }
                    let old = (*letter != 'A')
                        .then(|| {
                            exec::run_ok(
                                {
                                    let mut cmd = store.git_command(&repo);
                                    cmd.args(["show", &format!("HEAD:{p}")]);
                                    cmd
                                },
                                None,
                            )
                            .ok()
                            .map(|o| o.stdout)
                        })
                        .flatten();
                    let new = (*letter != 'D')
                        .then(|| {
                            fs::read(repo.join(p))
                                .ok()
                                .map(|b| String::from_utf8_lossy(&b).into_owned())
                        })
                        .flatten();
                    parts.push((p.as_str(), old, new));
                }
                // A variant file has several staged sources; each is shown under its own name.
                let one = parts.len() == 1;
                parts
                    .iter()
                    .map(|(p, old, new)| {
                        let label = if one { c.path.as_str() } else { *p };
                        unified(label, old.as_deref(), new.as_deref())
                    })
                    .collect::<String>()
            }
            _ => continue,
        };
        let notes: String = vchanges
            .iter()
            .filter(|v| v.path == c.path)
            .map(|v| {
                if v.secret {
                    format!(
                        "# secret {} changed (staging updates {} in the vault)\n",
                        v.name, v.updates
                    )
                } else {
                    format!(
                        "# variable {} changed: {} → {} (staging updates {})\n",
                        v.name, v.from, v.to, v.updates
                    )
                }
            })
            .collect();
        out.push(FileDiff {
            path: tilde(c),
            state: c.state,
            diff: notes + &crate::detect::redact(&diff, &known),
        });
    }
    Ok(out)
}

/// Run a plain unified diff through the bundled delta, returning ANSI-styled
/// text, or `None` when delta is unavailable or fails.
pub fn through_delta(
    store: &Store,
    plain: &str,
    cancel: Option<&exec::CancelToken>,
) -> Option<String> {
    let mut cmd = std::process::Command::new(store.tools.delta());
    cmd.env_clear()
        .args(["--paging=never", "--no-gitconfig", "--true-color=never"]);
    let out = exec::run_with_input(cmd, Some(plain.as_bytes().to_vec()), cancel).ok()?;
    (out.success() && !out.stdout.trim().is_empty()).then_some(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_cover_files_and_folders_only_on_boundaries() {
        assert!(covers(".config", ".config/app/x"));
        assert!(covers(".a", ".a"));
        assert!(!covers(".a", ".ab"));
    }

    #[test]
    fn unified_marks_new_and_deleted_files() {
        let n = unified("x", None, Some("hi\n"));
        assert!(n.contains("new file mode") && n.contains("--- /dev/null") && n.contains("+hi"));
        let d = unified("x", Some("hi\n"), None);
        assert!(
            d.contains("deleted file mode") && d.contains("+++ /dev/null") && d.contains("-hi")
        );
        assert!(unified("x", Some("a\0"), Some("b")).contains("Binary files"));
    }
}
