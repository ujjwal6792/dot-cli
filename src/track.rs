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

/// Express `norm` below `home` even when one of them is spelled through a
/// symlink (a symlinked HOME, `/tmp` vs `/private/tmp`): find the ancestor of
/// `norm` that is HOME itself and re-root the rest there. Symlinks *below* HOME
/// are deliberately not resolved.
fn map_into_home(norm: &Path, home: &Path) -> Option<PathBuf> {
    if norm.starts_with(home) {
        return Some(norm.to_path_buf());
    }
    let canonical_home = fs::canonicalize(home).ok()?;
    norm.ancestors()
        .filter(|a| !a.as_os_str().is_empty())
        .find(|a| fs::canonicalize(a).is_ok_and(|c| c == canonical_home))
        .map(|a| home.join(norm.strip_prefix(a).expect("an ancestor")))
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
    let abs = match map_into_home(&norm, home) {
        Some(abs) => abs,
        None => {
            return Err(usage(format!(
                "{arg:?} is outside HOME ({}); only HOME paths can be tracked",
                home.display()
            )));
        }
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
    // Case-insensitive filesystems resolve `.ZSHRC` to `.Zshrc`; the repository
    // would then record the wrong spelling. Insist on the on-disk name.
    let mut cur = home.clone();
    let mut real: Vec<String> = Vec::new();
    for c in &comps {
        let name = c.as_os_str();
        let mut spelled = name.to_string_lossy().into_owned();
        if fs::symlink_metadata(cur.join(name)).is_ok() {
            let exact = fs::read_dir(&cur)
                .map(|rd| rd.flatten().any(|e| e.file_name() == name))
                .unwrap_or(true);
            if !exact {
                let lower = spelled.to_lowercase();
                if let Some(actual) = fs::read_dir(&cur).ok().and_then(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .find(|n| n.to_lowercase() == lower)
                }) {
                    spelled = actual;
                }
                real.push(spelled);
                let mut full = real.join("/");
                let done = real.len();
                for rest in &comps[done..] {
                    full.push('/');
                    full.push_str(&rest.as_os_str().to_string_lossy());
                }
                return Err(usage(format!(
                    "~/{rel} is spelled ~/{full} on disk (this filesystem ignores case); \
                     use the exact spelling so the repository records the real name"
                )));
            }
        }
        real.push(spelled);
        cur.push(name);
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

pub(crate) fn refuse(message: String) -> anyhow::Error {
    DwError::new(Kind::Error, message).into()
}

fn unix_nanos(m: &fs::Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// FNV-1a, 64 bit: stable across Rust versions, which matters because guards
/// are persisted.
fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Files up to this size are hashed whole; larger ones by their first and last
/// 64 KiB (plus size and mtime), which keeps a guard cheap.
const FULL_HASH_LIMIT: u64 = 4 * 1024 * 1024;
const SAMPLE: u64 = 64 * 1024;

fn content_hash(path: &Path, len: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else {
        return "unreadable".into();
    };
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut buf = Vec::new();
    if len <= FULL_HASH_LIMIT {
        if f.read_to_end(&mut buf).is_err() {
            return "unreadable".into();
        }
        h = fnv1a(h, &buf);
    } else {
        buf.resize(SAMPLE as usize, 0);
        if f.read_exact(&mut buf).is_err() {
            return "unreadable".into();
        }
        h = fnv1a(h, &buf);
        if f.seek(SeekFrom::End(-(SAMPLE as i64))).is_err() || f.read_exact(&mut buf).is_err() {
            return "unreadable".into();
        }
        h = fnv1a(h, &buf);
    }
    format!("{h:016x}")
}

pub(crate) fn home_fingerprint(abs: &Path) -> String {
    let Ok(m) = fs::symlink_metadata(abs) else {
        return "missing".into();
    };
    let (kind, extra) = if m.file_type().is_symlink() {
        (
            "link",
            fs::read_link(abs)
                .map(|t| t.display().to_string())
                .unwrap_or_default(),
        )
    } else if m.is_dir() {
        ("dir", String::new())
    } else {
        ("file", content_hash(abs, m.len()))
    };
    format!(
        "{kind}:{}:{}:{:o}:{extra}",
        m.len(),
        unix_nanos(&m),
        m.mode() & 0o7777
    )
}

pub(crate) fn repo_fingerprint(repo: &Path, source: &str) -> String {
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
            assignments: keys,
            variants: with_variants,
            ..
        } => {
            let assigned = assignments(&repo)?;
            let vars = crate::variants::Variants::read(&repo)?;
            for p in with_variants {
                g.repo.insert(format!("variants:{p}"), vars.fingerprint(p));
            }
            for t in files {
                g.repo.insert(
                    format!("tracked:{t}"),
                    ctx.tracked
                        .files
                        .get(t)
                        .map_or("-".to_string(), |e| repo_fingerprint(&repo, &e.source)),
                );
            }
            for k in keys {
                g.repo
                    .insert(format!("assignment:{k}"), yes_no(assigned.contains(k)));
            }
            for f in forget_folders {
                g.repo.insert(
                    format!("folder:{f}"),
                    yes_no(ctx.tracked.folders.contains(f)),
                );
            }
        }
        Action::Discard { files, .. } => {
            for f in files {
                g.home.insert(f.clone(), home_fingerprint(&home.join(f)));
            }
        }
        Action::VariantAdd { path, .. } => {
            g.home
                .insert(path.clone(), home_fingerprint(&home.join(path)));
            g.repo.extend(crate::variants::guard(store, action)?);
        }
        Action::VariantRemove { .. } => {
            g.repo.extend(crate::variants::guard(store, action)?);
        }
        Action::ValueSet { .. } | Action::ValueUnset { .. } => {
            g.repo.extend(crate::values::guard(store, ctx, action)?);
        }
        Action::ValueWire { .. } => {
            g.repo.extend(crate::values::guard(store, ctx, action)?);
        }
        Action::Encrypt { path, .. } | Action::SecretMove { path, .. } => {
            g.home
                .insert(path.clone(), home_fingerprint(&home.join(path)));
            g.repo.extend(crate::detect::guard(store, ctx, action));
        }
        Action::RemoteAdd { name, .. } | Action::RemoteRemove { name } => {
            g.repo
                .insert(format!("remote:{name}"), remote_state(store, name));
        }
        Action::Push { remotes } => {
            for r in remotes {
                g.repo.insert(format!("remote:{r}"), remote_state(store, r));
            }
            g.repo.insert(
                "head:HEAD".into(),
                store
                    .git(&["rev-parse", "HEAD"])
                    .map_or("-".into(), |o| o.stdout.trim().to_string()),
            );
        }
        Action::HideAdd { pattern } | Action::HideRemove { pattern } => {
            g.repo.insert(
                format!("rule:{pattern}"),
                yes_no(hide::has_rule(&repo, pattern)?),
            );
        }
        Action::Assign { .. }
        | Action::MachineAdd { .. }
        | Action::MachineRename { .. }
        | Action::MachineRetire { .. } => {
            g.repo.extend(crate::machines::guard(store, action)?);
        }
    }
    Ok(g)
}

fn remote_state(store: &Store, name: &str) -> String {
    crate::remote::remote_url(store, name)
        .ok()
        .flatten()
        .unwrap_or_else(|| "-".into())
}

fn describe_key(home_side: bool, key: &str) -> String {
    if home_side {
        return format!("~/{key} in HOME");
    }
    match key.split_once(':') {
        Some(("tracked", p)) => format!("the repository entry for ~/{p}"),
        Some(("hidden", p)) => format!("the hide rules for ~/{p}"),
        Some(("folder", p)) => format!("the tracked folder ~/{p}"),
        Some(("assignment" | "assigned", p)) => format!("the assignment of ~/{p}"),
        Some(("variants", p)) => format!("the variants of ~/{p}"),
        Some(("registry", p)) => format!("the registry entry of {p}"),
        Some(("data", _)) => "the assignment and variant data".to_string(),
        Some(("rule", p)) => format!("the hide rule `{p}`"),
        Some(("remote", p)) => format!("the remote {p}"),
        Some(("head", _)) => "the repository's latest commit".to_string(),
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

pub(crate) fn new_item(
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
        ..Preview::default()
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
    let below = format!("{}/", hp.rel);
    let removed_assignments: Vec<String> = assigned
        .iter()
        .filter(|k| **k == hp.rel || k.starts_with(&below))
        .cloned()
        .collect();
    for k in &removed_assignments {
        data.push(format!("remove the assignment of ~/{k} from {DATA_FILE}"));
    }
    let mut notes =
        vec!["HOME files are kept on every machine; they just stop syncing".to_string()];
    if let Some(above) = ctx.tracked.folder_above(&hp.rel) {
        notes.push(format!(
            "~/{above} is a tracked folder: the untracked file shows as New there again"
        ));
    }
    let files: Vec<String> = entries.iter().map(|(t, _)| (*t).clone()).collect();
    let mut changes: Vec<FileChange> = entries
        .iter()
        .map(|(t, e)| FileChange {
            action: "remove".into(),
            target: (*t).clone(),
            source: e.source.clone(),
            kind: if e.symlink { "symlink" } else { "file" }.into(),
        })
        .collect();
    let vars = crate::variants::Variants::read(&store.repo())?;
    let with_variants: Vec<String> = files
        .iter()
        .filter(|t| !vars.get(t).is_empty())
        .cloned()
        .collect();
    for t in &with_variants {
        for f in vars.files_of(&store.repo(), t) {
            changes.push(FileChange {
                action: "remove".into(),
                target: t.clone(),
                source: f,
                kind: "file".into(),
            });
        }
        data.push(format!(
            "remove the {} variant(s) of ~/{t} from {DATA_FILE}",
            vars.get(t).len()
        ));
    }
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
        ..Preview::default()
    };
    let action = Action::Untrack {
        path: hp.rel.clone(),
        files,
        forget_folders,
        assignments: removed_assignments,
        variants: with_variants,
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

fn covers(path: &str, rel: &str) -> bool {
    path.len() < rel.len() && rel.starts_with(path) && rel.as_bytes()[path.len()] == b'/'
}

/// A pending track/untrack item for a folder above `rel` or for something below
/// it: toggling `rel` would overlap it.
fn overlapping_item<'a>(list: &'a pending::List, rel: &str) -> Option<&'a Item> {
    list.items.iter().find(|i| match &i.action {
        Action::Track { path, .. } | Action::Untrack { path, .. } => {
            covers(path, rel) || covers(rel, path)
        }
        _ => false,
    })
}

fn same_path_item<'a>(list: &'a pending::List, rel: &str) -> Option<&'a Item> {
    list.items.iter().find(|i| {
        matches!(&i.action, Action::Track { path, .. } | Action::Untrack { path, .. } if path == rel)
    })
}

enum Decision {
    Drop(u64),
    Add(Box<Item>),
}

/// The stale test used when adding: a pending item replaced by a fresh request.
pub fn stale_checker<'a>(store: &'a Store, ctx: &'a Ctx) -> impl Fn(&Item) -> bool + 'a {
    move |i| stale_reason(store, ctx, i).ok().flatten().is_some()
}

/// Space: drop the pending track/untrack item of `hp` if there is one, else add
/// the item that flips its tracked state. A folder holding tracked files offers
/// untrack of them. An item that would overlap a pending folder item (or sit
/// inside one) is refused instead of added. Planning, which runs chezmoi,
/// happens before the lock is taken; freshness is re-checked inside it.
pub fn toggle(store: &Store, hp: &HomePath, requester: &str) -> Result<Toggled> {
    for _ in 0..3 {
        let list = pending::load(store)?;
        let decision = if let Some(existing) = same_path_item(&list, &hp.rel) {
            Decision::Drop(existing.id.expect("stored"))
        } else {
            if let Some(o) = overlapping_item(&list, &hp.rel) {
                return Err(refuse(format!(
                    "~/{} overlaps pending item {} ({}); drop that item first",
                    hp.rel,
                    o.label(),
                    o.summary
                )));
            }
            let ctx = Ctx::load(store)?;
            let is_dir = fs::symlink_metadata(&hp.abs).is_ok_and(|m| m.is_dir());
            let untrack = if is_dir {
                ctx.tracked.folders.contains(&hp.rel) || ctx.tracked.under(&hp.rel).next().is_some()
            } else {
                ctx.tracked.is_tracked(&hp.rel)
            };
            Decision::Add(Box::new(if untrack {
                plan_untrack(store, &ctx, hp, requester)?
            } else {
                plan_track(store, &ctx, hp, requester)?
            }))
        };
        let lock = store.lock()?;
        let list = pending::load(store)?;
        match decision {
            Decision::Drop(id) => {
                if list.get(id).is_none() {
                    return Ok(Toggled {
                        message: format!("#{id} was already dropped"),
                        ..Toggled::default()
                    });
                }
                let mut gone = pending::drop_items(store, &lock, &[id])?;
                let item = gone.remove(0);
                return Ok(Toggled {
                    message: format!("Dropped {}: {}", item.label(), item.summary),
                    dropped: Some(item),
                    added: None,
                });
            }
            Decision::Add(item) => {
                if let Some(same) = same_path_item(&list, &hp.rel) {
                    return Ok(Toggled {
                        message: format!("Already pending as {}: {}", same.label(), same.summary),
                        ..Toggled::default()
                    });
                }
                if let Some(o) = overlapping_item(&list, &hp.rel) {
                    return Err(refuse(format!(
                        "~/{} overlaps pending item {} ({}); drop that item first",
                        hp.rel,
                        o.label(),
                        o.summary
                    )));
                }
                let ctx = Ctx::load(store)?;
                if stale_reason(store, &ctx, &item)?.is_some() {
                    continue; // the world moved while planning: plan again
                }
                let check = stale_checker(store, &ctx);
                let mut added = pending::add(store, &lock, vec![*item], &check)?;
                let item = added.remove(0).item;
                return Ok(Toggled {
                    message: format!("Added {}: {}", item.label(), item.summary),
                    added: Some(item),
                    dropped: None,
                });
            }
        }
    }
    Err(refuse(format!(
        "~/{} kept changing while it was being planned; try again",
        hp.rel
    )))
}

/// `i` in the TUI: hide the path with an anchored rule through a pending item;
/// pressing it again drops that item. For an already hidden path it offers to
/// remove the rule when that exact rule exists.
pub fn toggle_hide(store: &Store, hp: &HomePath, is_dir: bool, requester: &str) -> Result<Toggled> {
    let pattern = hide::pattern_for_path(&hp.rel, is_dir);
    let list = pending::load(store)?;
    let same = list.items.iter().find(|i| {
        matches!(&i.action, Action::HideAdd { pattern: p } | Action::HideRemove { pattern: p } if *p == pattern)
    });
    if let Some(existing) = same {
        let id = existing.id.expect("stored");
        let lock = store.lock()?;
        let mut gone = pending::drop_items(store, &lock, &[id])?;
        let item = gone.remove(0);
        return Ok(Toggled {
            message: format!("Dropped {}: {}", item.label(), item.summary),
            dropped: Some(item),
            added: None,
        });
    }
    let ctx = Ctx::load(store)?;
    let hidden = ctx.rules.is_hidden(&hp.abs, is_dir);
    let item = if hidden {
        if !hide::has_rule(&store.repo(), &pattern)? {
            let why = ctx
                .rules
                .why_hidden(&hp.abs, is_dir)
                .unwrap_or_else(|| "a hide rule".into());
            return Err(refuse(format!(
                "~/{} is hidden: {why}; edit the rules with `dw hide`",
                hp.rel
            )));
        }
        plan_hide(store, &ctx, false, &pattern, requester)?
    } else {
        plan_hide(store, &ctx, true, &pattern, requester)?
    };
    let lock = store.lock()?;
    recheck(store, &lock, std::slice::from_ref(&item))?;
    let ctx = Ctx::load(store)?;
    let check = stale_checker(store, &ctx);
    let mut added = pending::add(store, &lock, vec![item], &check)?;
    let item = added.remove(0).item;
    Ok(Toggled {
        message: format!("Added {}: {}", item.label(), item.summary),
        added: Some(item),
        dropped: None,
    })
}

/// Before taking the lock, items were planned from a snapshot; under the lock
/// they must still describe the world.
pub fn recheck(store: &Store, _lock: &Lock, items: &[Item]) -> Result<()> {
    let ctx = Ctx::load(store)?;
    for i in items {
        if let Some(reason) = stale_reason(store, &ctx, i)? {
            return Err(DwError::new(
                Kind::Stale,
                format!("{reason} while the request was being planned; run the command again"),
            )
            .into());
        }
    }
    Ok(())
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
    /// Targets an applied untrack removed; chezmoi's state for them is cleaned
    /// afterwards (see [`cleanup_state`]).
    #[serde(skip)]
    pub forgotten: Vec<String>,
}

pub(crate) fn git_literal(store: &Store, args: &[&OsStr]) -> Result<exec::Output> {
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
pub(crate) fn unstaged(store: &Store) -> Result<BTreeSet<String>> {
    Ok(repo_changes(store)?
        .into_iter()
        .filter(|(x, y, _)| *y != ' ' || *x == '?')
        .map(|(_, _, p)| p)
        .collect())
}

pub(crate) fn ensure_repo_clean(store: &Store) -> Result<()> {
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

pub(crate) fn stage(store: &Store, paths: &[String]) -> Result<()> {
    for chunk in paths.chunks(CHUNK) {
        let mut args: Vec<&OsStr> = vec!["add".as_ref(), "-A".as_ref(), "--".as_ref()];
        args.extend(chunk.iter().map(|p| OsStr::new(p.as_str())));
        git_literal(store, &args)?;
    }
    Ok(())
}

pub(crate) fn index_tree(store: &Store) -> Result<String> {
    Ok(git_literal(store, &["write-tree".as_ref()])?
        .stdout
        .trim()
        .to_string())
}

/// Return index and working tree to `tree`, discarding whatever the failed item did.
pub(crate) fn rollback(store: &Store, tree: &str) -> Result<()> {
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
pub(crate) fn verify_and_stage(
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
                store.chezmoi_locked(&args)?;
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
            assignments: keys,
            variants: with_variants,
            ..
        } => {
            crate::variants::remove_all(store, with_variants)?;
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
            if !keys.is_empty() {
                crate::repo::edit_data(&repo, |doc| {
                    let dw = crate::repo::dotwarden_table(doc)?;
                    if let Some(t) = dw.get_mut("assignments").and_then(|a| a.as_table_mut()) {
                        for k in keys {
                            t.remove(k);
                        }
                    }
                    Ok(())
                })?;
            }
            verify_and_stage(store, &expected_sources(item), &[DATA_FILE])
        }
        Action::Discard { .. }
        | Action::RemoteAdd { .. }
        | Action::RemoteRemove { .. }
        | Action::Push { .. } => {
            unreachable!("run by apply_items")
        }
        Action::HideAdd { pattern } => {
            hide::add_rule(&repo, pattern)?;
            verify_and_stage(store, &BTreeSet::from([hide::HIDE_FILE.to_string()]), &[])
        }
        Action::HideRemove { pattern } => {
            hide::remove_rule(&repo, pattern)?;
            verify_and_stage(store, &BTreeSet::from([hide::HIDE_FILE.to_string()]), &[])
        }
        Action::VariantAdd { path, target, .. } => {
            let expected = crate::variants::exec(store, &item.action)?;
            let staged = verify_and_stage(store, &expected, &[DATA_FILE])?;
            crate::variants::after_add(store, path, target)?;
            Ok(staged)
        }
        Action::VariantRemove { .. } => {
            let expected = crate::variants::exec(store, &item.action)?;
            verify_and_stage(store, &expected, &[DATA_FILE])
        }
        Action::ValueSet { .. } | Action::ValueUnset { .. } | Action::ValueWire { .. } => {
            let expected = crate::values::exec(store, &item.action)?;
            verify_and_stage(store, &expected, &[])
        }
        Action::Encrypt { .. } => {
            let expected = crate::encfile::exec(store, &item.action)?;
            verify_and_stage(store, &expected, &[])
        }
        Action::SecretMove { .. } => {
            let expected = crate::detect::exec(store, &item.action)?;
            let staged = verify_and_stage(store, &expected, &[])?;
            crate::detect::prune(store)?;
            Ok(staged)
        }
        Action::Assign { .. }
        | Action::MachineAdd { .. }
        | Action::MachineRename { .. }
        | Action::MachineRetire { .. } => {
            let expected = crate::machines::exec(store, &item.action)?;
            let staged = verify_and_stage(store, &expected, &[DATA_FILE])?;
            crate::machines::finish(store, &item.action)?;
            Ok(staged)
        }
    }
}

pub(crate) fn state_backup(store: &Store) -> Result<Option<Vec<u8>>> {
    match fs::read(store.dirs.chezmoi_state()) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("reading chezmoi's state"),
    }
}

/// Put chezmoi's persistent state back as it was before a failed track: `add`
/// records last-written state for every file it touched.
pub(crate) fn state_restore(store: &Store, backup: Option<Vec<u8>>) -> Result<()> {
    let _guard = store.chezmoi_guard()?;
    let path = store.dirs.chezmoi_state();
    match backup {
        Some(b) => crate::fsutil::write_atomic(&path, b),
        None => match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        },
    }
}

/// Apply `items` in order, each all-or-nothing. A stale item is skipped and a
/// failed one is rolled back (repository and chezmoi state); the others still
/// apply. The caller holds the lock.
pub fn apply_items(store: &Store, lock: &Lock, items: &[Item]) -> Result<Vec<ItemResult>> {
    ensure_repo_clean(store)?;
    let mut results = Vec::new();
    let mut tracked_now: Vec<String> = Vec::new();
    for item in items {
        let result = |status, message: Option<String>, staged| ItemResult {
            id: item.id,
            kind: item.action.kind_name(),
            summary: item.summary.clone(),
            status,
            message,
            staged,
            forgotten: Vec::new(),
        };
        let ctx = Ctx::load(store)?;
        if let Some(reason) = stale_reason(store, &ctx, item)? {
            results.push(result(Outcome::Stale, Some(reason), vec![]));
            continue;
        }
        let tree = index_tree(store)?;
        let tracks = matches!(
            item.action,
            Action::Track { .. } | Action::VariantAdd { .. }
        );
        let backup = if tracks { state_backup(store)? } else { None };
        let outcome = match &item.action {
            Action::Discard { files, .. } => {
                crate::changes::discard_files(store, files, &item.requester).map(|snap| {
                    let note = match snap {
                        Some(id) => format!("snapshot {id} taken first"),
                        None => "nothing to snapshot (file was deleted)".to_string(),
                    };
                    (Vec::new(), Some(note))
                })
            }
            Action::RemoteAdd { .. } | Action::RemoteRemove { .. } | Action::Push { .. } => {
                crate::remote::exec_action(store, &item.action).map(|note| (Vec::new(), Some(note)))
            }
            _ => exec_item(store, &ctx, item).map(|staged| (staged, None)),
        };
        match outcome {
            Ok((staged, note)) => {
                let mut r = result(Outcome::Applied, note, staged);
                if let Action::Untrack { files, .. } = &item.action {
                    r.forgotten = files.clone();
                }
                if let Action::Track { files, .. } = &item.action {
                    tracked_now.extend(files.iter().filter(|f| !f.symlink).map(|f| f.path.clone()));
                }
                results.push(r);
            }
            Err(e) => {
                let mut message = format!("{e:#}");
                if let Err(re) = rollback(store, &tree) {
                    message.push_str(&format!(
                        "; ROLLBACK FAILED ({re:#}); run `git reset --hard` and `git clean -fd` in {}",
                        store.repo().display()
                    ));
                }
                if tracks && let Err(re) = state_restore(store, backup) {
                    message.push_str(&format!("; chezmoi's state could not be restored ({re:#})"));
                }
                results.push(result(Outcome::Failed, Some(message), vec![]));
            }
        }
    }
    if !tracked_now.is_empty() {
        // Detection only proposes; a failure here must not fail the apply.
        let _ = crate::values::propose_wires(store, lock, &tracked_now);
    }
    Ok(results)
}

/// Delete chezmoi's last-written state of untracked targets (and of folders that
/// no longer hold tracked files). `chezmoi state delete` handles one key per
/// call, so this runs after the mutation lock was released, a few keys at a
/// time, re-checking under the lock that a key was not tracked again in between.
/// Returns warnings; cleaning is best-effort and never fails an apply.
pub fn cleanup_state(store: &Store, results: &[ItemResult]) -> Vec<String> {
    let targets: BTreeSet<&String> = results.iter().flat_map(|r| &r.forgotten).collect();
    if targets.is_empty() {
        return Vec::new();
    }
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for t in targets {
        keys.insert(t.clone());
        let mut cur = t.as_str();
        while let Some((parent, _)) = cur.rsplit_once('/') {
            keys.insert(parent.to_string());
            cur = parent;
        }
    }
    let keys: Vec<String> = keys.into_iter().collect();
    let mut warnings = Vec::new();
    for chunk in keys.chunks(25) {
        let done = (|| -> Result<()> {
            let _lock = store.lock()?;
            let tracked = source::load(&store.repo())?;
            for k in chunk {
                if tracked.under(k).next().is_some() {
                    continue;
                }
                let key = format!("--key={}", store.dirs.home.join(k).display());
                store.chezmoi_locked(&["state", "delete", "--bucket=entryState", key.as_str()])?;
            }
            Ok(())
        })();
        crate::lock::yield_to_waiters();
        if let Err(e) = done {
            warnings.push(format!("could not clean chezmoi's state: {e:#}"));
            break;
        }
    }
    warnings
}

/// Apply stored items by id (all when `ids` is `None`), removing those that
/// applied from the list. The lock is taken per item, so a long batch never
/// blocks other commands for longer than its largest item.
pub fn apply_pending(store: &Store, ids: Option<&[u64]>) -> Result<Vec<ItemResult>> {
    let list = pending::load(store)?;
    let chosen: Vec<u64> = match ids {
        None => list.items.iter().filter_map(|i| i.id).collect(),
        Some(ids) => {
            let mut v = Vec::new();
            for id in ids {
                list.get(*id).ok_or_else(|| pending::not_found(*id))?;
                v.push(*id);
            }
            v.sort();
            v.dedup();
            v
        }
    };
    let mut results = Vec::new();
    for id in chosen {
        let lock = store.lock()?;
        let Some(item) = pending::load(store)?.get(id).cloned() else {
            continue; // dropped by someone else meanwhile
        };
        let mut r = apply_items(store, &lock, std::slice::from_ref(&item))?;
        if r.iter().any(|x| x.status == Outcome::Applied) {
            pending::drop_items(store, &lock, &[id])?;
        }
        results.append(&mut r);
        drop(lock);
        crate::lock::yield_to_waiters();
    }
    let warnings = cleanup_state(store, &results);
    attach_warnings(&mut results, warnings);
    Ok(results)
}

/// Show state-cleaning warnings on the first applied untrack result.
fn attach_warnings(results: &mut [ItemResult], warnings: Vec<String>) {
    if warnings.is_empty() {
        return;
    }
    if let Some(r) = results.iter_mut().find(|r| !r.forgotten.is_empty()) {
        r.message = Some(warnings.join("; "));
    }
}

/// Apply freshly planned items immediately without storing them (lock per item).
pub fn apply_now(store: &Store, items: &[Item]) -> Result<Vec<ItemResult>> {
    let mut results = Vec::new();
    for item in items {
        let lock = store.lock()?;
        results.append(&mut apply_items(store, &lock, std::slice::from_ref(item))?);
        drop(lock);
        crate::lock::yield_to_waiters();
    }
    let warnings = cleanup_state(store, &results);
    attach_warnings(&mut results, warnings);
    Ok(results)
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
