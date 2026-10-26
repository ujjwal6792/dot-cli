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
    let args = ["dump", "--format", "json", "--include=files,symlinks"];
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
    let out = store.chezmoi(&["status", "--include=files,symlinks"])?;
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
    targets: BTreeMap<String, Target>,
}

/// Classify every change on this machine.
pub fn scan(store: &Store, ctx: &Ctx) -> Result<Scan> {
    let home = &store.dirs.home;
    let repo = store.repo();
    let targets = targets(store)?;
    let columns = status_columns(store)?;
    let last = last_written(store)?;
    let mut changes = Vec::new();

    let mut staged: BTreeMap<String, &'static str> = BTreeMap::new();
    for (letter, path) in staged_paths(store)? {
        match decode_source_path(&repo, &path) {
            Some((target, _)) => {
                let present = fs::symlink_metadata(repo.join(&path)).is_ok();
                if present && !targets.contains_key(&target) {
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
    Ok(Scan { changes, targets })
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
}

/// Stage HOME content of the selected changes (`None` selects everything that
/// can be staged). Explicitly named changes that cannot be staged are refused;
/// with `None`, or inside a named folder, they are skipped and reported.
pub fn stage(store: &Store, sel: Option<&[String]>, dry_run: bool) -> Result<Done> {
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    let ctx = Ctx::load(store)?;
    let scan = scan(store, &ctx)?;
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut refusals = Vec::new();
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
            _ if c.kind == "deleted" => Some((
                Kind::Error,
                "deleted in HOME; stage the deletion with `dw untrack`".into(),
            )),
            _ if ctx
                .tracked
                .files
                .get(&c.path)
                .is_some_and(|e| e.source.ends_with(".tmpl")) =>
            {
                Some((
                    Kind::Error,
                    "is a template; staging templates needs references (not supported yet)".into(),
                ))
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
    if files.is_empty() && skipped.is_empty() {
        bail!("nothing to stage");
    }
    if !dry_run && !files.is_empty() {
        track::ensure_repo_clean(store)?;
        let tree = track::index_tree(store)?;
        let backup = track::state_backup(store)?;
        let run = || -> Result<()> {
            for chunk in files.chunks(CHUNK) {
                let mut args: Vec<std::ffi::OsString> = vec!["add".into()];
                args.extend(chunk.iter().map(|f| store.dirs.home.join(f).into()));
                store.chezmoi_locked(&args)?;
            }
            let changed: Vec<String> = track::unstaged(store)?.into_iter().collect();
            track::stage(store, &changed)?;
            if !track::unstaged(store)?.is_empty() {
                bail!("some changes could not be staged");
            }
            Ok(())
        };
        if let Err(e) = run() {
            track::rollback(store, &tree)?;
            track::state_restore(store, backup)?;
            return Err(e.context("staging failed; the repository was left as before"));
        }
    }
    Ok(Done {
        paths: files.iter().map(|f| track::tilde(f)).collect(),
        skipped,
        dry_run,
    })
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
        .filter(|p| match decode_source_path(&repo, p) {
            Some((t, _)) => selected_targets.contains(t.as_str()),
            None => repo_paths.contains(p.as_str()),
        })
        .collect();
    let done = Done {
        paths: changes.iter().map(tilde).collect(),
        skipped: Vec::new(),
        dry_run,
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
fn restore_last_written(store: &Store, targets: &BTreeSet<&str>) -> Result<()> {
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
    }
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
                let old = scan.targets.get(&c.path).map(|t| t.text.as_str());
                let new = home_text(&store.dirs.home.join(&c.path));
                unified(&c.path, old, new.as_deref())
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
                let mut old = None;
                let mut new = None;
                for (letter, p) in &staged {
                    if decode_source_path(&repo, p).is_none_or(|(t, _)| t != c.path) {
                        continue;
                    }
                    if *letter != 'A' {
                        old = exec::run_ok(
                            {
                                let mut cmd = store.git_command(&repo);
                                cmd.args(["show", &format!("HEAD:{p}")]);
                                cmd
                            },
                            None,
                        )
                        .ok()
                        .map(|o| o.stdout);
                    }
                    if *letter != 'D' {
                        new = fs::read(repo.join(p))
                            .ok()
                            .map(|b| String::from_utf8_lossy(&b).into_owned());
                    }
                }
                unified(&c.path, old.as_deref(), new.as_deref())
            }
            _ => continue,
        };
        out.push(FileDiff {
            path: tilde(c),
            state: c.state,
            diff,
        });
    }
    Ok(out)
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
