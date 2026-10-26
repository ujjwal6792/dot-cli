//! Tracking and untracking HOME paths, and hide-rule changes: resolving paths,
//! planning pending items with an exact preview, checking their freshness and
//! applying them (spec §5, §8).
//!
//! Applying an item is all-or-nothing: the index tree is recorded first and the
//! repository is reset to it when anything fails. The result is staged in git
//! (`git add`); committing belongs to the change model.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::error::{DwError, Kind};
use crate::exec::{self, CancelToken};
use crate::hide::{self, HideRules};
use crate::lock::Lock;
use crate::pending::{
    self, Action, FileChange, Guard, Item, Preview, TrackFile, now_rfc3339, tilde_rel,
};
use crate::repo::DATA_FILE;
use crate::source::{self, TrackState, Tracked};
use crate::store::Store;

/// Paths handed to one chezmoi or git invocation.
const CHUNK: usize = 400;

/// What planning and applying need to know about the repository and hide rules.
pub struct Ctx {
    pub rules: HideRules,
    pub tracked: Tracked,
}

impl Ctx {
    pub fn load(store: &Store) -> Result<Self> {
        Ok(Self {
            rules: HideRules::for_store(store)?,
            tracked: source::load(&store.repo())?,
        })
    }
}

/// A path inside HOME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomePath {
    pub abs: PathBuf,
    /// Relative to HOME, `/`-separated, valid UTF-8.
    pub rel: String,
}

fn lexical_normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve `~/x`, an absolute path or one relative to `cwd` to a path under HOME.
/// Refuses paths outside HOME, HOME itself, Dotwarden's own directories, names
/// that cannot be represented and paths below a symlinked folder.
pub fn resolve_home_path(store: &Store, arg: &str, cwd: &Path) -> Result<HomePath> {
    Ok(resolve_inner(store, arg, cwd, false)?.expect("HOME itself is refused"))
}

/// Like [`resolve_home_path`] but HOME itself is allowed and yields `None`.
pub fn resolve_home_or_root(store: &Store, arg: &str, cwd: &Path) -> Result<Option<HomePath>> {
    resolve_inner(store, arg, cwd, true)
}

fn resolve_inner(
    store: &Store,
    arg: &str,
    cwd: &Path,
    allow_root: bool,
) -> Result<Option<HomePath>> {
    let usage = |m: String| -> anyhow::Error { DwError::usage(m).into() };
    let home = &store.dirs.home;
    let raw = if arg == "~" {
        home.clone()
    } else if let Some(r) = arg.strip_prefix("~/") {
        home.join(r)
    } else if arg.starts_with('~') {
        return Err(usage(format!(
            "{arg:?}: only `~` and `~/path` are understood, not other users' homes"
        )));
    } else if Path::new(arg).is_absolute() {
        PathBuf::from(arg)
    } else {
        cwd.join(arg)
    };
    let norm = lexical_normalise(&raw);
    let canonical_home = fs::canonicalize(home).unwrap_or_else(|_| home.clone());
    let abs = if norm.starts_with(home) {
        norm
    } else if let Ok(rest) = norm.strip_prefix(&canonical_home) {
        home.join(rest)
    } else {
        return Err(usage(format!(
            "{arg:?} is outside HOME ({}); only HOME paths can be tracked",
            home.display()
        )));
    };
    let rel_path = abs.strip_prefix(home).expect("checked above");
    if rel_path.as_os_str().is_empty() && allow_root {
        return Ok(None);
    }
    if rel_path.as_os_str().is_empty() {
        return Err(usage(
            "HOME itself cannot be tracked; name a file or a folder inside it".to_string(),
        ));
    }
    let Some(rel) = rel_path.to_str().map(str::to_string) else {
        return Err(usage(format!(
            "{arg:?}: the name is not valid UTF-8 and cannot be tracked"
        )));
    };
    if crate::assign::validate_target_path(&rel).is_err() {
        return Err(usage(format!(
            "cannot track {rel:?}: the name contains a newline or another control character \
             that cannot be represented"
        )));
    }
    if store.dirs.own_dirs().iter().any(|d| abs.starts_with(d)) {
        return Err(usage(format!(
            "~/{rel} is inside one of dotwarden's own directories and cannot be tracked"
        )));
    }
    let mut cur = home.clone();
    let comps: Vec<_> = rel_path.components().collect();
    for c in &comps[..comps.len() - 1] {
        cur.push(c);
        if fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(usage(format!(
                "~/{rel} is below the symlink {}; track the symlink itself or use the real path",
                cur.display()
            )));
        }
    }
    Ok(Some(HomePath { abs, rel }))
}

// ---- folders -----------------------------------------------------------

/// The files of a folder that can be tracked.
#[derive(Debug, Default)]
pub struct Walk {
    pub files: Vec<TrackFile>,
    /// Entries skipped because a hide rule matches.
    pub hidden: usize,
    /// Nested `.git` folders, sockets and other entries that cannot be tracked.
    pub skipped: usize,
}

/// Enumerate the files and symlinks below `rel`, skipping hidden entries.
pub fn walk_folder(
    home: &Path,
    rel: &str,
    rules: &HideRules,
    cancel: Option<&CancelToken>,
) -> Result<Walk> {
    let mut w = Walk::default();
    walk_into(home, rel, rules, cancel, &mut w)?;
    w.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(w)
}

fn walk_into(
    home: &Path,
    rel: &str,
    rules: &HideRules,
    cancel: Option<&CancelToken>,
    w: &mut Walk,
) -> Result<()> {
    if cancel.is_some_and(CancelToken::is_cancelled) {
        bail!("cancelled");
    }
    let dir = home.join(rel);
    let rd = fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?;
    for item in rd.flatten() {
        let name = item.file_name();
        let path = item.path();
        let Ok(ft) = item.file_type() else {
            w.skipped += 1;
            continue;
        };
        let Some(child) = name.to_str().map(|n| format!("{rel}/{n}")) else {
            w.skipped += 1;
            continue;
        };
        if rules.is_hidden(&path, ft.is_dir()) {
            w.hidden += 1;
        } else if ft.is_dir() {
            if name == ".git" || crate::assign::validate_target_path(&child).is_err() {
                w.skipped += 1;
            } else {
                walk_into(home, &child, rules, cancel, w)?;
            }
        } else if (ft.is_file() || ft.is_symlink())
            && crate::assign::validate_target_path(&child).is_ok()
        {
            w.files.push(TrackFile {
                path: child,
                symlink: ft.is_symlink(),
            });
        } else {
            w.skipped += 1;
        }
    }
    Ok(())
}

/// Untracked, unhidden files inside tracked folders: the New changes.
pub fn new_files(
    home: &Path,
    tracked: &Tracked,
    rules: &HideRules,
    cancel: Option<&CancelToken>,
) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    for folder in &tracked.folders {
        if tracked.folder_above(folder).is_some() || !home.join(folder).is_dir() {
            continue;
        }
        for f in walk_folder(home, folder, rules, cancel)?.files {
            if !tracked.is_tracked(&f.path) {
                out.insert(f.path);
            }
        }
    }
    Ok(out.into_iter().collect())
}

// ---- planning ----------------------------------------------------------

fn refuse(message: String) -> anyhow::Error {
    DwError::new(Kind::Error, message).into()
}

fn unix_nanos(m: &fs::Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

fn home_fingerprint(abs: &Path) -> String {
    let Ok(m) = fs::symlink_metadata(abs) else {
        return "missing".into();
    };
    let kind = if m.file_type().is_symlink() {
        "link"
    } else if m.is_dir() {
        "dir"
    } else {
        "file"
    };
    let link = if m.file_type().is_symlink() {
        fs::read_link(abs)
            .map(|t| t.display().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "{kind}:{}:{}:{:o}:{link}",
        m.len(),
        unix_nanos(&m),
        m.mode() & 0o7777
    )
}

fn repo_fingerprint(repo: &Path, source: &str) -> String {
    match fs::metadata(repo.join(source)) {
        Ok(m) => format!("{source}:{}:{}", m.len(), unix_nanos(&m)),
        Err(_) => "-".into(),
    }
}

fn assignments(repo: &Path) -> Result<BTreeSet<String>> {
    let text = match fs::read_to_string(repo.join(DATA_FILE)) {
        Ok(t) => t,
        Err(_) => return Ok(BTreeSet::new()),
    };
    let data: toml::Table = toml::from_str(&text)?;
    Ok(data
        .get("dotwarden")
        .and_then(|d| d.get("assignments"))
        .and_then(toml::Value::as_table)
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default())
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}

/// Everything the outcome of `action` depends on, as it is right now.
pub fn compute_guard(store: &Store, ctx: &Ctx, action: &Action) -> Result<Guard> {
    let repo = store.repo();
    let home = &store.dirs.home;
    let mut g = Guard::default();
    match action {
        Action::Track {
            path,
            record_folder,
            files,
            ..
        } => {
            for f in files {
                g.home
                    .insert(f.path.clone(), home_fingerprint(&home.join(&f.path)));
                g.repo.insert(
                    format!("tracked:{}", f.path),
                    ctx.tracked
                        .files
                        .get(&f.path)
                        .map_or("-".to_string(), |e| e.source.clone()),
                );
                g.repo.insert(
                    format!("hidden:{}", f.path),
                    yes_no(ctx.rules.is_hidden(&home.join(&f.path), false)),
                );
            }
            if *record_folder {
                g.repo.insert(
                    format!("folder:{path}"),
                    yes_no(ctx.tracked.folders.contains(path)),
                );
            }
        }
        Action::Untrack {
            files,
            forget_folders,
            ..
        } => {
            let assigned = assignments(&repo)?;
            for t in files {
                g.repo.insert(
                    format!("tracked:{t}"),
                    ctx.tracked
                        .files
                        .get(t)
                        .map_or("-".to_string(), |e| repo_fingerprint(&repo, &e.source)),
                );
                g.repo
                    .insert(format!("assignment:{t}"), yes_no(assigned.contains(t)));
            }
            for f in forget_folders {
                g.repo.insert(
                    format!("folder:{f}"),
                    yes_no(ctx.tracked.folders.contains(f)),
                );
            }
        }
        Action::HideAdd { pattern } | Action::HideRemove { pattern } => {
            g.repo.insert(
                format!("rule:{pattern}"),
                yes_no(hide::has_rule(&repo, pattern)?),
            );
        }
    }
    Ok(g)
}

fn describe_key(home_side: bool, key: &str) -> String {
    if home_side {
        return format!("~/{key} in HOME");
    }
    match key.split_once(':') {
        Some(("tracked", p)) => format!("the repository entry for ~/{p}"),
        Some(("hidden", p)) => format!("the hide rules for ~/{p}"),
        Some(("folder", p)) => format!("the tracked folder ~/{p}"),
        Some(("assignment", p)) => format!("the assignment of ~/{p}"),
        Some(("rule", p)) => format!("the hide rule `{p}`"),
        _ => key.to_string(),
    }
}

fn differences(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
    home_side: bool,
) -> Vec<String> {
    old.keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|k| old.get(*k) != new.get(*k))
        .map(|k| describe_key(home_side, k))
        .collect()
}

/// `None` when `item` still applies as previewed; otherwise why not.
pub fn stale_reason(store: &Store, ctx: &Ctx, item: &Item) -> Result<Option<String>> {
    let now = compute_guard(store, ctx, &item.action)?;
    if now == item.guard {
        return Ok(None);
    }
    let mut diff = differences(&item.guard.home, &now.home, true);
    diff.extend(differences(&item.guard.repo, &now.repo, false));
    let more = diff.len().saturating_sub(3);
    diff.truncate(3);
    let mut s = format!("{} changed since this item was added", diff.join(", "));
    if more > 0 {
        s.push_str(&format!(" (and {more} more)"));
    }
    Ok(Some(s))
}

/// Parse `chezmoi add --dry-run --verbose` output into `(source path, is_dir)`.
fn parse_dry_run(out: &str) -> Result<Vec<(String, bool)>> {
    let mut found: Vec<(String, Option<bool>)> = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            let n = rest.len().saturating_sub(3) / 2;
            let ok = rest.len() == 2 * n + 3
                && rest.is_char_boundary(n)
                && rest[n..].starts_with(" b/")
                && rest[n + 3..] == rest[..n];
            if !ok {
                bail!("cannot read chezmoi's preview line {line:?}");
            }
            found.push((rest[..n].to_string(), None));
        } else if let Some(mode) = line.strip_prefix("new file mode ")
            && let Some(last) = found.last_mut()
        {
            last.1 = Some(mode.starts_with("40"));
        }
    }
    found
        .into_iter()
        .map(|(s, dir)| match dir {
            Some(d) => Ok((s, d)),
            None => Err(refuse(format!(
                "chezmoi would modify the existing repository file {s}; refusing to guess"
            ))),
        })
        .collect()
}

/// Ask chezmoi exactly which repository files adding `files` would create.
fn preview_add(store: &Store, files: &[TrackFile]) -> Result<Vec<FileChange>> {
    let repo = store.repo();
    let mut out = Vec::new();
    for chunk in files.chunks(CHUNK) {
        let mut args: Vec<std::ffi::OsString> =
            vec!["add".into(), "--dry-run".into(), "--verbose".into()];
        args.extend(chunk.iter().map(|f| store.dirs.home.join(&f.path).into()));
        let o = store.chezmoi(&args)?;
        for (source, is_dir) in parse_dry_run(&o.stdout)? {
            if is_dir {
                continue;
            }
            let (target, symlink) = source::decode_source_path(&repo, &source)
                .with_context(|| format!("chezmoi would write {source}, which is not a target"))?;
            out.push(FileChange {
                action: "add".into(),
                target,
                source,
                kind: if symlink { "symlink" } else { "file" }.into(),
            });
        }
    }
    out.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(out)
}

fn new_item(
    store: &Store,
    requester: &str,
    action: Action,
    summary: String,
    preview: Preview,
    ctx: &Ctx,
) -> Result<Item> {
    let guard = compute_guard(store, ctx, &action)?;
    Ok(Item {
        id: None,
        action,
        requester: requester.to_string(),
        created: now_rfc3339(),
        summary,
        preview,
        guard,
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Plan tracking `hp`: a file, a symlink or a folder.
pub fn plan_track(store: &Store, ctx: &Ctx, hp: &HomePath, requester: &str) -> Result<Item> {
    let meta = fs::symlink_metadata(&hp.abs)
        .map_err(|_| refuse(format!("~/{} does not exist", hp.rel)))?;
    let is_dir = meta.is_dir();
    if let Some(why) = ctx.rules.why_hidden(&hp.abs, is_dir) {
        return Err(refuse(format!(
            "~/{} is hidden and cannot be tracked: {why} (see `dw hide`)",
            hp.rel
        )));
    }
    let mut notes = Vec::new();
    let (folder, record_folder, files, summary) = if is_dir {
        let walk = walk_folder(&store.dirs.home, &hp.rel, &ctx.rules, None)?;
        let total = walk.files.len();
        let files: Vec<TrackFile> = walk
            .files
            .into_iter()
            .filter(|f| !ctx.tracked.is_tracked(&f.path))
            .collect();
        let record =
            ctx.tracked.folder_above(&hp.rel).is_none() && !ctx.tracked.folders.contains(&hp.rel);
        if files.is_empty() && !record {
            return Err(refuse(format!(
                "nothing to track in ~/{}: it is already a tracked folder and has no new files",
                hp.rel
            )));
        }
        if total > files.len() {
            notes.push(format!(
                "{} already tracked, left unchanged",
                plural(total - files.len(), "file is", "files are")
            ));
        }
        if walk.hidden > 0 {
            notes.push(format!(
                "{} skipped because of hide rules",
                plural(walk.hidden, "entry", "entries")
            ));
        }
        if walk.skipped > 0 {
            notes.push(format!(
                "{} skipped (nested .git folders, special files, unrepresentable names)",
                plural(walk.skipped, "entry", "entries")
            ));
        }
        if let Some(above) = ctx.tracked.folder_above(&hp.rel) {
            notes.push(format!(
                "~/{above} is already a tracked folder; new files in ~/{} show as New",
                hp.rel
            ));
        } else {
            notes.push("new files that appear in this folder later show as New; nothing is added automatically".into());
        }
        let summary = format!(
            "Track folder ~/{} ({})",
            hp.rel,
            plural(files.len(), "file", "files")
        );
        (true, record, files, summary)
    } else if meta.is_file() || meta.file_type().is_symlink() {
        if ctx.tracked.is_tracked(&hp.rel) {
            return Err(refuse(format!("~/{} is already tracked", hp.rel)));
        }
        let symlink = meta.file_type().is_symlink();
        let summary = format!(
            "Track ~/{}{}",
            hp.rel,
            if symlink { " (symlink)" } else { "" }
        );
        if let Some(above) = ctx.tracked.folder_above(&hp.rel) {
            notes.push(format!("inside the tracked folder ~/{above}"));
        }
        (
            false,
            false,
            vec![TrackFile {
                path: hp.rel.clone(),
                symlink,
            }],
            summary,
        )
    } else {
        return Err(refuse(format!(
            "~/{} is not a regular file, symlink or folder and cannot be tracked",
            hp.rel
        )));
    };
    let changes = preview_add(store, &files)?;
    let want: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let got: BTreeSet<&str> = changes.iter().map(|c| c.target.as_str()).collect();
    if want != got {
        bail!("chezmoi's preview does not match the files to track; refusing to continue");
    }
    let mut data = Vec::new();
    if record_folder {
        data.push(format!(
            "record ~/{} as a tracked folder in {DATA_FILE}",
            hp.rel
        ));
    }
    let preview = Preview {
        applies_to: Some("all machines".into()),
        files: changes,
        data,
        rules: Vec::new(),
        notes,
    };
    let action = Action::Track {
        path: hp.rel.clone(),
        folder,
        record_folder,
        files,
    };
    new_item(store, requester, action, summary, preview, ctx)
}

/// Plan untracking `hp`: a tracked file or symlink, or everything tracked below a folder.
pub fn plan_untrack(store: &Store, ctx: &Ctx, hp: &HomePath, requester: &str) -> Result<Item> {
    let entries: Vec<(&String, &source::SourceEntry)> = ctx.tracked.under(&hp.rel).collect();
    let forget_folders: Vec<String> = ctx.tracked.folders_under(&hp.rel).cloned().collect();
    if entries.is_empty() && forget_folders.is_empty() {
        return Err(refuse(format!("~/{} is not tracked", hp.rel)));
    }
    let assigned = assignments(&store.repo())?;
    let mut data = Vec::new();
    for f in &forget_folders {
        data.push(format!(
            "stop recording ~/{f} as a tracked folder in {DATA_FILE}"
        ));
    }
    for (t, _) in &entries {
        if assigned.contains(*t) {
            data.push(format!("remove the assignment of ~/{t} from {DATA_FILE}"));
        }
    }
    let mut notes =
        vec!["HOME files are kept on every machine; they just stop syncing".to_string()];
    if let Some(above) = ctx.tracked.folder_above(&hp.rel) {
        notes.push(format!(
            "~/{above} is a tracked folder: the untracked file shows as New there again"
        ));
    }
    let files: Vec<String> = entries.iter().map(|(t, _)| (*t).clone()).collect();
    let changes = entries
        .iter()
        .map(|(t, e)| FileChange {
            action: "remove".into(),
            target: (*t).clone(),
            source: e.source.clone(),
            kind: if e.symlink { "symlink" } else { "file" }.into(),
        })
        .collect();
    let is_folder =
        !forget_folders.is_empty() || entries.len() > 1 || !ctx.tracked.is_tracked(&hp.rel);
    let summary = if is_folder {
        format!(
            "Untrack ~/{} ({})",
            hp.rel,
            plural(files.len(), "file", "files")
        )
    } else {
        format!("Untrack ~/{}", hp.rel)
    };
    let preview = Preview {
        applies_to: None,
        files: changes,
        data,
        rules: Vec::new(),
        notes,
    };
    let action = Action::Untrack {
        path: hp.rel.clone(),
        files,
        forget_folders,
    };
    new_item(store, requester, action, summary, preview, ctx)
}

/// Plan adding or removing a hide rule.
pub fn plan_hide(
    store: &Store,
    ctx: &Ctx,
    add: bool,
    pattern: &str,
    requester: &str,
) -> Result<Item> {
    let repo = store.repo();
    let present = hide::has_rule(&repo, pattern)?;
    let mut preview = Preview::default();
    if add {
        hide::validate_pattern(pattern)?;
        if present {
            return Err(refuse(format!("`{pattern}` is already a hide rule")));
        }
        preview
            .rules
            .push(format!("add `{pattern}` to {}", hide::HIDE_FILE));
        let mut text = fs::read_to_string(repo.join(hide::HIDE_FILE)).unwrap_or_default();
        text.push_str(&format!("\n{pattern}\n"));
        let rules = HideRules::parse(&store.dirs.home, &text)?;
        let matching: Vec<&String> = ctx
            .tracked
            .files
            .keys()
            .filter(|t| rules.is_hidden(&store.dirs.home.join(t), false))
            .collect();
        if !matching.is_empty() {
            let shown: Vec<String> = matching.iter().take(5).map(|t| format!("~/{t}")).collect();
            preview.notes.push(format!(
                "{} tracked by this rule stay tracked ({}{}); hiding only keeps new paths from being tracked",
                plural(matching.len(), "file", "files"),
                shown.join(", "),
                if matching.len() > 5 { ", …" } else { "" }
            ));
        }
    } else {
        if !present {
            return Err(refuse(format!("`{pattern}` is not a hide rule")));
        }
        preview
            .rules
            .push(format!("remove `{pattern}` from {}", hide::HIDE_FILE));
    }
    let action = if add {
        Action::HideAdd {
            pattern: pattern.to_string(),
        }
    } else {
        Action::HideRemove {
            pattern: pattern.to_string(),
        }
    };
    let summary = format!(
        "{} hide rule `{pattern}`",
        if add { "Add" } else { "Remove" }
    );
    new_item(store, requester, action, summary, preview, ctx)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Track,
    Untrack,
}

/// Plan every path; if any is refused nothing is planned and all reasons are reported.
pub fn plan_paths(
    store: &Store,
    plan: Plan,
    args: &[String],
    cwd: &Path,
    requester: &str,
) -> Result<Vec<Item>> {
    let ctx = Ctx::load(store)?;
    let mut items: Vec<Item> = Vec::new();
    let mut errors: Vec<(Kind, String)> = Vec::new();
    for arg in args {
        let result = resolve_home_path(store, arg, cwd).and_then(|hp| match plan {
            Plan::Track => plan_track(store, &ctx, &hp, requester),
            Plan::Untrack => plan_untrack(store, &ctx, &hp, requester),
        });
        match result {
            Ok(item) => {
                if !items.iter().any(|i| i.action.key() == item.action.key()) {
                    items.push(item);
                }
            }
            Err(e) => errors.push(crate::error::classify(&e)),
        }
    }
    if let Some((kind, _)) = errors.first() {
        let message = errors
            .iter()
            .map(|(_, m)| m.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(DwError::new(*kind, message).into());
    }
    Ok(items)
}

// ---- toggling (TUI Space) ------------------------------------------------

#[derive(Debug, Default)]
pub struct Toggled {
    pub message: String,
    pub added: Option<Item>,
    pub dropped: Option<Item>,
}

/// Space: drop the pending track/untrack item of `hp` if there is one, else add
/// the item that flips its tracked state. Decided under the lock so quick
/// repeats and concurrent CLI use stay consistent.
pub fn toggle(store: &Store, lock: &Lock, hp: &HomePath, requester: &str) -> Result<Toggled> {
    let list = pending::load(store)?;
    if let Some(existing) = list.items.iter().find(|i| {
        matches!(&i.action, Action::Track { path, .. } | Action::Untrack { path, .. } if *path == hp.rel)
    }) {
        let id = existing.id.expect("stored");
        let mut gone = pending::drop_items(store, lock, &[id])?;
        let item = gone.remove(0);
        return Ok(Toggled {
            message: format!("Dropped {}: {}", item.label(), item.summary),
            dropped: Some(item),
            added: None,
        });
    }
    let ctx = Ctx::load(store)?;
    let is_dir = fs::symlink_metadata(&hp.abs).is_ok_and(|m| m.is_dir());
    let untrack = if is_dir {
        ctx.tracked.folders.contains(&hp.rel)
    } else {
        ctx.tracked.is_tracked(&hp.rel)
    };
    let item = if untrack {
        plan_untrack(store, &ctx, hp, requester)?
    } else {
        plan_track(store, &ctx, hp, requester)?
    };
    let mut added = pending::add(store, lock, vec![item])?;
    let item = added.remove(0).item;
    Ok(Toggled {
        message: format!("Added {}: {}", item.label(), item.summary),
        added: Some(item),
        dropped: None,
    })
}

// ---- applying ------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemResult {
    pub id: Option<u64>,
    pub kind: &'static str,
    pub summary: String,
    pub status: Outcome,
    pub message: Option<String>,
    /// Repository paths staged in git.
    pub staged: Vec<String>,
}

fn git_literal(store: &Store, args: &[&OsStr]) -> Result<exec::Output> {
    let mut cmd = store.git_command(&store.repo());
    cmd.env("GIT_LITERAL_PATHSPECS", "1").args(args);
    exec::run_ok(cmd, None)
}

/// Changed paths of the repository as `(X, Y, path)` from `git status`.
fn repo_changes(store: &Store) -> Result<Vec<(char, char, String)>> {
    let o = git_literal(
        store,
        &[
            "--no-optional-locks".as_ref(),
            "status".as_ref(),
            "--porcelain=v1".as_ref(),
            "-z".as_ref(),
            "--untracked-files=all".as_ref(),
            "--no-renames".as_ref(),
        ],
    )?;
    Ok(o.stdout
        .split('\0')
        .filter(|e| e.len() > 3)
        .map(|e| {
            let mut c = e.chars();
            (c.next().unwrap(), c.next().unwrap(), e[3..].to_string())
        })
        .collect())
}

/// Paths changed in the working tree relative to the index (what is not staged).
fn unstaged(store: &Store) -> Result<BTreeSet<String>> {
    Ok(repo_changes(store)?
        .into_iter()
        .filter(|(x, y, _)| *y != ' ' || *x == '?')
        .map(|(_, _, p)| p)
        .collect())
}

fn ensure_repo_clean(store: &Store) -> Result<()> {
    let dirty = unstaged(store)?;
    if dirty.is_empty() {
        return Ok(());
    }
    let shown: Vec<_> = dirty.iter().take(5).cloned().collect();
    Err(refuse(format!(
        "the repository has changes that are not staged ({}{}); dotwarden keeps the repository \
         identical to its index, so resolve them in {} first",
        shown.join(", "),
        if dirty.len() > 5 { ", …" } else { "" },
        store.repo().display()
    )))
}

fn stage(store: &Store, paths: &[String]) -> Result<()> {
    for chunk in paths.chunks(CHUNK) {
        let mut args: Vec<&OsStr> = vec!["add".as_ref(), "-A".as_ref(), "--".as_ref()];
        args.extend(chunk.iter().map(|p| OsStr::new(p.as_str())));
        git_literal(store, &args)?;
    }
    Ok(())
}

fn index_tree(store: &Store) -> Result<String> {
    Ok(git_literal(store, &["write-tree".as_ref()])?
        .stdout
        .trim()
        .to_string())
}

/// Return index and working tree to `tree`, discarding whatever the failed item did.
fn rollback(store: &Store, tree: &str) -> Result<()> {
    git_literal(store, &["add".as_ref(), "-A".as_ref()])?;
    git_literal(
        store,
        &[
            "read-tree".as_ref(),
            "--reset".as_ref(),
            "-u".as_ref(),
            OsStr::new(tree),
        ],
    )?;
    Ok(())
}

/// Check that exactly the previewed repository paths changed, then stage them.
fn verify_and_stage(
    store: &Store,
    expected: &BTreeSet<String>,
    optional: &[&str],
) -> Result<Vec<String>> {
    let actual = unstaged(store)?;
    let unexpected: Vec<_> = actual
        .iter()
        .filter(|p| !expected.contains(*p) && !optional.contains(&p.as_str()))
        .collect();
    let missing: Vec<_> = expected.iter().filter(|p| !actual.contains(*p)).collect();
    if !unexpected.is_empty() || !missing.is_empty() {
        bail!(
            "the repository changed differently than previewed (unexpected: {unexpected:?}, missing: {missing:?})"
        );
    }
    let paths: Vec<String> = actual.into_iter().collect();
    stage(store, &paths)?;
    if !unstaged(store)?.is_empty() {
        bail!("some changes could not be staged");
    }
    Ok(paths)
}

fn expected_sources(item: &Item) -> BTreeSet<String> {
    item.preview
        .files
        .iter()
        .map(|f| f.source.clone())
        .collect()
}

fn exec_item(store: &Store, ctx: &Ctx, item: &Item) -> Result<Vec<String>> {
    let repo = store.repo();
    let home = &store.dirs.home;
    match &item.action {
        Action::Track {
            path,
            record_folder,
            files,
            ..
        } => {
            for f in files {
                if ctx.rules.is_hidden(&home.join(&f.path), false) {
                    bail!("~/{} is hidden by a hide rule", f.path);
                }
            }
            for chunk in files.chunks(CHUNK) {
                let mut args: Vec<std::ffi::OsString> = vec!["add".into()];
                args.extend(chunk.iter().map(|f| home.join(&f.path).into()));
                store.chezmoi(&args)?;
            }
            if *record_folder {
                let mut folders = source::read_folders(&repo)?;
                folders.insert(path.clone());
                source::write_folders(&repo, &folders)?;
            }
            verify_and_stage(store, &expected_sources(item), &[DATA_FILE])
        }
        Action::Untrack {
            files,
            forget_folders,
            ..
        } => {
            let root = repo.join(source::source_root(&repo));
            for t in files {
                let entry = ctx
                    .tracked
                    .files
                    .get(t)
                    .with_context(|| format!("~/{t} is no longer tracked"))?;
                let file = repo.join(&entry.source);
                fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
                let mut dir = file.parent();
                while let Some(d) = dir {
                    if d == root || !d.starts_with(&root) || fs::remove_dir(d).is_err() {
                        break;
                    }
                    dir = d.parent();
                }
            }
            if !forget_folders.is_empty() {
                let mut folders = source::read_folders(&repo)?;
                for f in forget_folders {
                    folders.remove(f);
                }
                source::write_folders(&repo, &folders)?;
            }
            let assigned = assignments(&repo)?;
            let drop: Vec<&String> = files.iter().filter(|t| assigned.contains(*t)).collect();
            if !drop.is_empty() {
                crate::repo::edit_data(&repo, |doc| {
                    let dw = crate::repo::dotwarden_table(doc)?;
                    if let Some(t) = dw.get_mut("assignments").and_then(|a| a.as_table_mut()) {
                        for k in &drop {
                            t.remove(k);
                        }
                    }
                    Ok(())
                })?;
            }
            verify_and_stage(store, &expected_sources(item), &[DATA_FILE])
        }
        Action::HideAdd { pattern } => {
            hide::add_rule(&repo, pattern)?;
            verify_and_stage(store, &BTreeSet::from([hide::HIDE_FILE.to_string()]), &[])
        }
        Action::HideRemove { pattern } => {
            hide::remove_rule(&repo, pattern)?;
            verify_and_stage(store, &BTreeSet::from([hide::HIDE_FILE.to_string()]), &[])
        }
    }
}

/// Apply `items` in order, each all-or-nothing. A stale item is skipped and a
/// failed one is rolled back; the others still apply. The caller holds the lock.
pub fn apply_items(store: &Store, _lock: &Lock, items: &[Item]) -> Result<Vec<ItemResult>> {
    ensure_repo_clean(store)?;
    let mut results = Vec::new();
    for item in items {
        let result = |status, message: Option<String>, staged| ItemResult {
            id: item.id,
            kind: item.action.kind_name(),
            summary: item.summary.clone(),
            status,
            message,
            staged,
        };
        let ctx = Ctx::load(store)?;
        if let Some(reason) = stale_reason(store, &ctx, item)? {
            results.push(result(Outcome::Stale, Some(reason), vec![]));
            continue;
        }
        let tree = index_tree(store)?;
        match exec_item(store, &ctx, item) {
            Ok(staged) => results.push(result(Outcome::Applied, None, staged)),
            Err(e) => {
                let mut message = format!("{e:#}");
                if let Err(re) = rollback(store, &tree) {
                    message.push_str(&format!(
                        "; ROLLBACK FAILED ({re:#}); run `git reset --hard` and `git clean -fd` in {}",
                        store.repo().display()
                    ));
                }
                results.push(result(Outcome::Failed, Some(message), vec![]));
            }
        }
    }
    Ok(results)
}

/// Apply stored items by id (all when `ids` is `None`), removing those that
/// applied from the list. Takes the lock itself.
pub fn apply_pending(store: &Store, ids: Option<&[u64]>) -> Result<Vec<ItemResult>> {
    let lock = store.lock()?;
    let list = pending::load(store)?;
    let chosen: Vec<Item> = match ids {
        None => list.items.clone(),
        Some(ids) => {
            let mut v = Vec::new();
            for id in ids {
                v.push(
                    list.get(*id)
                        .ok_or_else(|| pending::not_found(*id))?
                        .clone(),
                );
            }
            v.sort_by_key(|i| i.id);
            v.dedup_by_key(|i| i.id);
            v
        }
    };
    let results = apply_items(store, &lock, &chosen)?;
    let done: Vec<u64> = results
        .iter()
        .filter(|r| r.status == Outcome::Applied)
        .filter_map(|r| r.id)
        .collect();
    if !done.is_empty() {
        pending::drop_items(store, &lock, &done)?;
    }
    Ok(results)
}

/// Apply freshly planned items immediately without storing them.
pub fn apply_now(store: &Store, items: &[Item]) -> Result<Vec<ItemResult>> {
    let lock = store.lock()?;
    apply_items(store, &lock, items)
}

/// The exit kind for a set of results: stale wins over a plain failure.
pub fn result_kind(results: &[ItemResult]) -> Option<Kind> {
    if results.iter().any(|r| r.status == Outcome::Stale) {
        Some(Kind::Stale)
    } else if results.iter().any(|r| r.status == Outcome::Failed) {
        Some(Kind::Error)
    } else {
        None
    }
}

// ---- ls --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LsEntry {
    pub name: String,
    /// Relative to HOME.
    pub path: String,
    /// `file`, `dir`, `symlink` or `other`.
    pub kind: &'static str,
    /// `tracked`, `tracked-folder`, `contains-tracked`, `new`, `untracked` or `hidden`.
    pub state: &'static str,
    pub hidden: bool,
    /// A pending `track` or `untrack` item for this path.
    pub pending: Option<&'static str>,
}

pub fn state_label(s: TrackState) -> &'static str {
    match s {
        TrackState::Tracked => "tracked",
        TrackState::TrackedFolder => "tracked-folder",
        TrackState::ContainsTracked => "contains-tracked",
        TrackState::New => "new",
        TrackState::Untracked => "untracked",
    }
}

pub fn ls_entry(
    home: &Path,
    ctx: &Ctx,
    marks: &BTreeMap<String, &'static str>,
    rel: &str,
) -> LsEntry {
    let abs = home.join(rel);
    let meta = fs::symlink_metadata(&abs).ok();
    let is_link = meta.as_ref().is_some_and(|m| m.file_type().is_symlink());
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    let kind = if is_link {
        "symlink"
    } else if is_dir {
        "dir"
    } else if meta.as_ref().is_some_and(|m| m.is_file()) {
        "file"
    } else {
        "other"
    };
    let hidden = ctx.rules.is_hidden(&abs, is_dir);
    let state = if hidden {
        "hidden"
    } else {
        state_label(ctx.tracked.state_of(rel, is_dir))
    };
    LsEntry {
        name: rel.rsplit('/').next().unwrap_or(rel).to_string(),
        path: rel.to_string(),
        kind,
        state,
        hidden,
        pending: marks.get(rel).copied(),
    }
}

/// Entries of the folder `hp` (or the single entry when it is not a folder),
/// sorted by name; hidden entries only with `all`.
pub fn ls(store: &Store, hp: Option<&HomePath>, all: bool) -> Result<Vec<LsEntry>> {
    let ctx = Ctx::load(store)?;
    let marks = pending::marks(&pending::load(store)?);
    let home = &store.dirs.home;
    let mut out = Vec::new();
    let dir_rel = hp.map(|h| h.rel.as_str());
    let abs = hp.map_or(home.clone(), |h| h.abs.clone());
    let meta = fs::symlink_metadata(&abs)
        .map_err(|_| refuse(format!("{} does not exist", abs.display())))?;
    if !meta.is_dir() {
        out.push(ls_entry(home, &ctx, &marks, dir_rel.unwrap_or_default()));
        return Ok(out);
    }
    for item in fs::read_dir(&abs)?.flatten() {
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let rel = match dir_rel {
            Some(d) => format!("{d}/{name}"),
            None => name,
        };
        let e = ls_entry(home, &ctx, &marks, &rel);
        if all || !e.hidden {
            out.push(e);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `~/rel` form for messages.
pub fn tilde(rel: &str) -> String {
    tilde_rel(rel)
}
