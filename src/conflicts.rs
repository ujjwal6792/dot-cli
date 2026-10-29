//! Conflicts (spec §5): a tracked file changed both in HOME and by an incoming
//! commit. The three inputs are base (what was last applied), local (HOME) and
//! incoming (the newly committed, rendered content). They are merged line by
//! line in memory; the live dotfile never receives conflict markers. A result is
//! written to HOME after a snapshot, made the file's last-written content and
//! staged, so the file stops being a conflict.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use similar::{DiffTag, TextDiff};

use crate::changes::{self, State};
use crate::error::{DwError, Kind};
use crate::lock::Lock;
use crate::pending::{Action, FileChange, Item, Preview};
use crate::snapshot;
use crate::store::Store;
use crate::track::{self, Ctx, refuse};

/// How many commits are searched for the base content.
const HISTORY: usize = 60;

// ---- merge engine ---------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub base: Vec<String>,
    pub local: Vec<String>,
    pub incoming: Vec<String>,
    /// First base line of the hunk (1-based); 0 when there is no base.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    /// Unchanged on both sides.
    Same(Vec<String>),
    /// Changed on one side only, or identically on both: merged automatically.
    Merged(Vec<String>),
    Conflict(Hunk),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Local,
    Incoming,
    Both,
}

fn lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(String::from).collect()
}

struct Edit {
    start: usize,
    end: usize,
    new: Vec<String>,
}

fn edits(base: &str, new: &str) -> Vec<Edit> {
    let new_lines = lines(new);
    TextDiff::from_lines(base, new)
        .ops()
        .iter()
        .filter_map(|op| {
            let (tag, o, n) = op.as_tag_tuple();
            (tag != DiffTag::Equal).then(|| Edit {
                start: o.start,
                end: o.end,
                new: new_lines[n].to_vec(),
            })
        })
        .collect()
}

fn side(base: &[String], start: usize, end: usize, edits: &[Edit]) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = start;
    for e in edits {
        out.extend_from_slice(&base[pos..e.start]);
        out.extend_from_slice(&e.new);
        pos = e.end;
    }
    out.extend_from_slice(&base[pos..end]);
    out
}

/// Three-way merge by lines. Without a base every difference is a conflict.
pub fn merge(base: Option<&str>, local: &str, incoming: &str) -> Vec<Chunk> {
    let Some(base_text) = base else {
        let (l, i) = (lines(local), lines(incoming));
        return TextDiff::from_lines(local, incoming)
            .ops()
            .iter()
            .map(|op| {
                let (tag, o, n) = op.as_tag_tuple();
                if tag == DiffTag::Equal {
                    Chunk::Same(l[o].to_vec())
                } else {
                    Chunk::Conflict(Hunk {
                        base: Vec::new(),
                        local: l[o].to_vec(),
                        incoming: i[n].to_vec(),
                        line: 0,
                    })
                }
            })
            .collect();
    };
    let b = lines(base_text);
    let (le, ie) = (edits(base_text, local), edits(base_text, incoming));
    let (mut i, mut j, mut pos) = (0, 0, 0);
    let mut chunks = Vec::new();
    loop {
        let next = match (le.get(i), ie.get(j)) {
            (None, None) => break,
            (Some(a), None) => a.start,
            (None, Some(c)) => c.start,
            (Some(a), Some(c)) => a.start.min(c.start),
        };
        if next > pos {
            chunks.push(Chunk::Same(b[pos..next].to_vec()));
        }
        let (start, mut end, i0, j0) = (next, next, i, j);
        loop {
            let before = (i, j);
            while let Some(a) = le.get(i).filter(|a| a.start <= end) {
                end = end.max(a.end);
                i += 1;
            }
            while let Some(c) = ie.get(j).filter(|c| c.start <= end) {
                end = end.max(c.end);
                j += 1;
            }
            if (i, j) == before {
                break;
            }
        }
        let mine = side(&b, start, end, &le[i0..i]);
        let theirs = side(&b, start, end, &ie[j0..j]);
        chunks.push(if i == i0 {
            Chunk::Merged(theirs)
        } else if j == j0 || mine == theirs {
            Chunk::Merged(mine)
        } else {
            Chunk::Conflict(Hunk {
                base: b[start..end].to_vec(),
                local: mine,
                incoming: theirs,
                line: start + 1,
            })
        });
        pos = end;
    }
    if pos < b.len() {
        chunks.push(Chunk::Same(b[pos..].to_vec()));
    }
    chunks
}

pub fn hunks(chunks: &[Chunk]) -> Vec<&Hunk> {
    chunks
        .iter()
        .filter_map(|c| match c {
            Chunk::Conflict(h) => Some(h),
            _ => None,
        })
        .collect()
}

fn push_lines(out: &mut String, ls: &[String]) {
    for l in ls {
        out.push_str(l);
        if !l.ends_with('\n') {
            out.push('\n');
        }
    }
}

/// The merged text with the given picks (`None` picks are written as conflict
/// markers, for the editor only). Lines are copied as they are.
fn assemble(chunks: &[Chunk], picks: &[Option<Pick>]) -> String {
    let mut out = String::new();
    let mut k = 0;
    for c in chunks {
        match c {
            Chunk::Same(ls) | Chunk::Merged(ls) => out.extend(ls.iter().map(String::as_str)),
            Chunk::Conflict(h) => {
                let pick = picks.get(k).copied().flatten();
                k += 1;
                match pick {
                    Some(Pick::Local) => out.extend(h.local.iter().map(String::as_str)),
                    Some(Pick::Incoming) => out.extend(h.incoming.iter().map(String::as_str)),
                    Some(Pick::Both) => {
                        push_lines(&mut out, &h.local);
                        out.extend(h.incoming.iter().map(String::as_str));
                    }
                    None => {
                        out.push_str(LOCAL_MARK);
                        push_lines(&mut out, &h.local);
                        out.push_str("=======\n");
                        push_lines(&mut out, &h.incoming);
                        out.push_str(INCOMING_MARK);
                    }
                }
            }
        }
    }
    out
}

const LOCAL_MARK: &str = "<<<<<<< local\n";
const INCOMING_MARK: &str = ">>>>>>> incoming\n";

/// The merge with every hunk decided; `None` while one is missing.
pub fn result(chunks: &[Chunk], picks: &[Option<Pick>]) -> Option<String> {
    let n = hunks(chunks).len();
    (picks.len() == n && picks.iter().all(Option::is_some)).then(|| assemble(chunks, picks))
}

/// The text the editor starts from: decided hunks resolved, others as markers.
pub fn with_markers(chunks: &[Chunk], picks: &[Option<Pick>]) -> String {
    assemble(chunks, picks)
}

pub fn has_markers(text: &str) -> bool {
    text.lines()
        .any(|l| l == LOCAL_MARK.trim_end() || l == INCOMING_MARK.trim_end())
}

// ---- inputs ---------------------------------------------------------------

pub struct Loaded {
    pub rel: String,
    pub chunks: Vec<Chunk>,
    /// Whether the last applied content was found (else every difference conflicts).
    pub has_base: bool,
    pub local: String,
    pub incoming: String,
}

impl Loaded {
    pub fn hunk_count(&self) -> usize {
        hunks(&self.chunks).len()
    }

    /// The merge when no hunk conflicts.
    pub fn clean(&self) -> Option<String> {
        self.result_with(&[])
    }

    fn result_with(&self, picks: &[Option<Pick>]) -> Option<String> {
        result(&self.chunks, picks)
    }

    /// Where the conflicting hunks are, without their content.
    pub fn describe(&self) -> String {
        let hs = hunks(&self.chunks);
        let places: Vec<String> = hs
            .iter()
            .enumerate()
            .map(|(i, h)| {
                if h.line > 0 {
                    format!("{} (base line {})", i + 1, h.line)
                } else {
                    format!("{}", i + 1)
                }
            })
            .collect();
        format!(
            "{} overlapping hunk(s): {}{}",
            hs.len(),
            places.join(", "),
            if self.has_base {
                ""
            } else {
                "; the last applied version is unknown, so every difference is listed"
            }
        )
    }
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Loaded(~/{}, {} hunk(s))", self.rel, self.hunk_count())
    }
}

/// Display text of `text` with secret values hidden.
pub fn masked(store: &Store, text: &str) -> String {
    crate::detect::redact(text, &crate::detect::known_pairs(store))
}

fn check_conflict(store: &Store, rel: &str) -> Result<changes::Scan> {
    let scan = changes::scan(store, &Ctx::load(store)?)?;
    if let Some(why) = scan.clashes.get(rel) {
        return Err(refuse(format!("~/{rel}: {why}")));
    }
    if scan.locked.contains(rel) {
        return Err(DwError::new(
            Kind::VaultLocked,
            format!(
                "~/{rel} needs the vault, which is locked; run `dw vault unlock` in a terminal"
            ),
        )
        .into());
    }
    if !scan
        .changes
        .iter()
        .any(|c| c.path == rel && c.state == State::Conflict && c.kind != "repo")
    {
        return Err(refuse(format!("~/{rel} is not a conflict")));
    }
    Ok(scan)
}

fn git_out(store: &Store, args: &[&str]) -> Option<String> {
    let mut cmd = store.git_command(&store.repo());
    cmd.env("GIT_LITERAL_PATHSPECS", "1").args(args);
    crate::exec::run_ok(cmd, None).ok().map(|o| o.stdout)
}

/// The content this machine last applied, found among the committed versions
/// of the file by its hash. Template sources and variant files with references
/// are rendered with the current values, which matches while those are unchanged.
fn find_base(store: &Store, ctx: &Ctx, rel: &str, sha: &str) -> Option<String> {
    let mut spec: Vec<String> = Vec::new();
    if let Some(e) = ctx.tracked.files.get(rel) {
        spec.push(e.source.clone());
    }
    spec.push(format!("{}/", crate::variants::repo_path(rel)));
    let n = HISTORY.to_string();
    let mut log = vec!["log", "--format=%H", "-n", &n, "--"];
    log.extend(spec.iter().map(String::as_str));
    let commits = git_out(store, &log)?;
    let mut seen = BTreeSet::new();
    for c in commits.lines() {
        let mut ls = vec!["ls-tree", "-r", c, "--"];
        ls.extend(spec.iter().map(String::as_str));
        let Some(tree) = git_out(store, &ls) else {
            continue;
        };
        for l in tree.lines() {
            // "<mode> blob <id>\t<path>"
            let Some((meta, path)) = l.split_once('\t') else {
                continue;
            };
            let Some(id) = meta.split_whitespace().nth(2) else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let Some(text) = git_out(store, &["cat-file", "blob", id]) else {
                continue;
            };
            if changes::sha256(text.as_bytes()) == sha {
                return Some(text);
            }
            if (path.ends_with(".tmpl") || text.contains("{{"))
                && let Ok(o) = store.chezmoi(&["execute-template", text.as_str()])
                && changes::sha256(o.stdout.as_bytes()) == sha
            {
                return Some(o.stdout);
            }
        }
    }
    None
}

fn local_text(store: &Store, rel: &str) -> Result<(String, u32)> {
    let p = store.dirs.home.join(rel);
    let meta = fs::symlink_metadata(&p)
        .map_err(|_| refuse(format!("~/{rel} is missing; use --take local or incoming")))?;
    if !meta.is_file() {
        return Err(refuse(format!(
            "~/{rel} is not a regular file; use --take local or incoming"
        )));
    }
    let text = fs::read_to_string(&p)
        .ok()
        .filter(|t| !t.contains('\0'))
        .ok_or_else(|| {
            refuse(format!(
                "~/{rel} is not a text file; use --take local or incoming"
            ))
        })?;
    Ok((text, meta.permissions().mode() & 0o777))
}

/// Read the three inputs of the conflict at `rel` and merge them.
pub fn load(store: &Store, rel: &str) -> Result<Loaded> {
    let scan = check_conflict(store, rel)?;
    let target = scan
        .targets
        .get(rel)
        .ok_or_else(|| refuse(format!("~/{rel} is not received on this machine")))?;
    if target.link {
        return Err(refuse(format!(
            "~/{rel} is a symlink; use --take local or incoming"
        )));
    }
    let (local, _) = local_text(store, rel)?;
    let ctx = Ctx::load(store)?;
    let key = store.dirs.home.join(rel).to_string_lossy().into_owned();
    let base = changes::last_written(store)?
        .get(&key)
        .and_then(|(_, sha)| find_base(store, &ctx, rel, sha));
    let chunks = merge(base.as_deref(), &local, &target.text);
    Ok(Loaded {
        rel: rel.to_string(),
        chunks,
        has_base: base.is_some(),
        local,
        incoming: target.text.clone(),
    })
}

// ---- resolving --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    Local,
    Incoming,
    /// The automatic merge; fails when hunks overlap.
    Merge,
    /// A finished text (hunk picks or an edited merge file).
    Text(String),
}

#[derive(Debug, Serialize)]
pub struct Resolved {
    pub path: String,
    /// `local`, `incoming` or `merge`.
    pub resolution: &'static str,
    pub snapshot: Option<String>,
}

fn write_private_keep_mode(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().context("no parent directory")?;
    let tmp = parent.join(format!(".dw-resolve.{}", std::process::id()));
    let r = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r.with_context(|| format!("writing {}", path.display()))
}

/// Make the committed version the last-written content of `rel` (so a HOME
/// file that differs from it is a plain Local change), then stage HOME.
fn stage_local(store: &Store, lock: &Lock, rel: &str) -> Result<()> {
    let backup = track::state_backup(store)?;
    let run = || -> Result<()> {
        changes::restore_last_written(store, &BTreeSet::from([rel]))?;
        changes::stage_with(store, Some(&[rel.to_string()]), false, Some(lock))?;
        Ok(())
    };
    if let Err(e) = run() {
        track::state_restore(store, backup)?;
        return Err(e.context(format!("~/{rel} was not resolved")));
    }
    Ok(())
}

/// Resolve the conflict at `rel` while holding the mutation lock.
pub fn resolve_locked(
    store: &Store,
    lock: &Lock,
    rel: &str,
    how: How,
    actor: &str,
) -> Result<Resolved> {
    let scan = check_conflict(store, rel)?;
    let incoming = scan.targets.get(rel).map(|t| t.text.clone());
    let how = match how {
        How::Merge => {
            let l = load(store, rel)?;
            match l.clean() {
                Some(t) => How::Text(t),
                None => {
                    return Err(
                        DwError::new(Kind::Conflict, format!("~/{rel}: {}", l.describe())).into(),
                    );
                }
            }
        }
        How::Text(t) if incoming.as_deref() == Some(t.as_str()) => How::Incoming,
        h => h,
    };
    let name = match &how {
        How::Local => "local",
        How::Incoming => "incoming",
        _ => "merge",
    };
    let mut out = Resolved {
        path: track::tilde(rel),
        resolution: name,
        snapshot: None,
    };
    let abs = store.dirs.home.join(rel);
    match how {
        How::Local => stage_local(store, lock, rel)?,
        How::Incoming => {
            let backup = track::state_backup(store)?;
            out.snapshot =
                snapshot::take(store, &[rel.to_string()], "resolve", actor)?.map(|s| s.id);
            let args: [&OsStr; 4] = [
                "apply".as_ref(),
                "--force".as_ref(),
                "--parent-dirs".as_ref(),
                abs.as_os_str(),
            ];
            if let Err(e) = store.chezmoi_locked(&args) {
                track::state_restore(store, backup)?;
                return Err(e.context(format!("~/{rel} was not resolved")));
            }
        }
        How::Text(text) => {
            let (before, mode) = match fs::read(&abs) {
                Ok(b) => (Some(b), fs::metadata(&abs)?.permissions().mode() & 0o777),
                Err(_) => (None, 0o644),
            };
            if before.as_deref() == Some(text.as_bytes()) {
                stage_local(store, lock, rel)?;
            } else {
                out.snapshot =
                    snapshot::take(store, &[rel.to_string()], "resolve", actor)?.map(|s| s.id);
                write_private_keep_mode(&abs, text.as_bytes(), mode)?;
                if let Err(e) = stage_local(store, lock, rel) {
                    if let Some(b) = &before {
                        let _ = write_private_keep_mode(&abs, b, mode);
                    }
                    return Err(e);
                }
            }
        }
        How::Merge => unreachable!("turned into a text above"),
    }
    Ok(out)
}

pub fn resolve(store: &Store, rel: &str, how: How, actor: &str) -> Result<Resolved> {
    let lock = store.lock()?;
    let r = resolve_locked(store, &lock, rel, how, actor)?;
    drop(lock);
    Ok(r)
}

/// The pending item for an agent's request to take the incoming version (or the
/// automatic merge, which must be clean) of the conflict at `rel`.
pub fn plan_resolve(store: &Store, rel: &str, merge: bool, requester: &str) -> Result<Item> {
    let ctx = Ctx::load(store)?;
    if merge {
        let l = load(store, rel)?;
        if l.clean().is_none() {
            return Err(DwError::new(
                Kind::Conflict,
                format!(
                    "~/{rel}: {}; an agent cannot choose between them",
                    l.describe()
                ),
            )
            .into());
        }
    } else {
        check_conflict(store, rel)?;
    }
    let preview = Preview {
        applies_to: None,
        files: vec![FileChange {
            action: "overwrite".into(),
            target: rel.to_string(),
            source: String::new(),
            kind: "file".into(),
        }],
        data: Vec::new(),
        rules: Vec::new(),
        notes: vec!["a snapshot of the current file is taken first (`dw snapshot list`)".into()],
        ..Preview::default()
    };
    let summary = format!(
        "Resolve the conflict at {} with the {} (requested by {requester})",
        track::tilde(rel),
        if merge {
            "automatic merge"
        } else {
            "incoming version"
        }
    );
    let action = Action::Resolve {
        path: rel.to_string(),
        merge,
    };
    track::new_item(store, requester, action, summary, preview, &ctx)
}

// ---- editor -----------------------------------------------------------------

/// A private directory with the merge file and read-only copies of both sides.
/// It is removed when dropped.
pub struct EditSession {
    dir: PathBuf,
    pub merge: PathBuf,
    local: PathBuf,
    incoming: PathBuf,
}

impl EditSession {
    pub fn new(store: &Store, loaded: &Loaded, picks: &[Option<Pick>]) -> Result<Self> {
        let dir = store
            .dirs
            .state_dir()
            .join(format!("merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let s = Self {
            merge: dir.join("merge"),
            local: dir.join("local"),
            incoming: dir.join("incoming"),
            dir,
        };
        let put = |p: &Path, text: &str| -> Result<()> {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(p)?;
            f.write_all(text.as_bytes())?;
            Ok(())
        };
        put(&s.merge, &with_markers(&loaded.chunks, picks))?;
        put(&s.local, &loaded.local)?;
        put(&s.incoming, &loaded.incoming)?;
        Ok(s)
    }

    /// The editor command: `$VISUAL` / `$EDITOR`, with nvim in diff mode.
    pub fn command(&self, store: &Store) -> std::process::Command {
        let editor = ["VISUAL", "EDITOR"]
            .iter()
            .find_map(|k| store.env().get(OsStr::new(k)).filter(|v| !v.is_empty()))
            .cloned()
            .unwrap_or_else(|| "vi".into());
        let name = Path::new(&editor)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut script = editor;
        let mut cmd = std::process::Command::new("sh");
        if name == "nvim" {
            script.push(" -d \"$1\" \"$2\" \"$3\"");
            cmd.arg("-c")
                .arg(script)
                .arg("sh")
                .args([&self.merge, &self.local, &self.incoming]);
        } else {
            script.push(" \"$1\"");
            cmd.arg("-c").arg(script).arg("sh").arg(&self.merge);
        }
        cmd.env_clear().envs(store.env());
        cmd
    }

    /// The edited merge file; fails while conflict markers remain.
    pub fn read_result(&self) -> Result<String> {
        let text = fs::read_to_string(&self.merge).context("reading the merge file")?;
        if has_markers(&text) {
            bail!("conflict markers are still in the merge file; nothing was written");
        }
        Ok(text)
    }
}

impl Drop for EditSession {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Edit the merge in `$EDITOR` (CLI, interactive) and return the result.
pub fn edit(store: &Store, loaded: &Loaded) -> Result<String> {
    let s = EditSession::new(store, loaded, &vec![None; loaded.hunk_count()])?;
    let status = s.command(store).status().context("starting the editor")?;
    if !status.success() {
        bail!("the editor exited with {status}; nothing was written");
    }
    s.read_result()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(chunks: &[Chunk], picks: &[Option<Pick>]) -> String {
        assemble(chunks, picks)
    }

    #[test]
    fn distant_edits_merge_cleanly() {
        let base = "a\nb\nc\nd\ne\n";
        let c = merge(Some(base), "a\nB\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
        assert!(hunks(&c).is_empty());
        assert_eq!(text(&c, &[]), "a\nB\nc\nd\nE\n");
    }

    #[test]
    fn overlapping_edits_conflict_and_picks_apply() {
        let base = "a\nb\nc\nd\ne\n";
        let c = merge(Some(base), "a\nL\nc\nd\nL2\n", "a\nI\nc\nd\nI2\n");
        assert_eq!(hunks(&c).len(), 2);
        assert_eq!(hunks(&c)[0].line, 2);
        let picks = [Some(Pick::Local), Some(Pick::Incoming)];
        assert_eq!(text(&c, &picks), "a\nL\nc\nd\nI2\n");
        let both = [Some(Pick::Both), Some(Pick::Both)];
        assert_eq!(text(&c, &both), "a\nL\nI\nc\nd\nL2\nI2\n");
        assert!(result(&c, &[Some(Pick::Local), None]).is_none());
    }

    #[test]
    fn identical_edits_and_no_base() {
        let c = merge(Some("a\nb\n"), "a\nX\n", "a\nX\n");
        assert!(hunks(&c).is_empty());
        let c = merge(None, "a\nL\nc\n", "a\nI\nc\n");
        assert_eq!(hunks(&c).len(), 1);
        assert_eq!(hunks(&c)[0].line, 0);
    }

    #[test]
    fn markers_only_for_undecided_hunks() {
        let c = merge(Some("a\nb\nc\n"), "L\nb\nL2\n", "I\nb\nI2\n");
        let t = with_markers(&c, &[Some(Pick::Incoming), None]);
        assert!(t.starts_with("I\nb\n<<<<<<< local\nL2\n=======\nI2\n>>>>>>> incoming\n"));
        assert!(has_markers(&t));
        assert!(!has_markers("I\nb\nI2\n"));
    }
}
