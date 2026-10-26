//! Snapshots (spec §11): a local copy of HOME files taken before dotwarden
//! overwrites them, so a discard or restore can always be undone.
//!
//! Layout: `$XDG_STATE_HOME/dotwarden/snapshots/<id>/meta.json` plus one blob
//! per regular file. Directories are 0700 and blobs 0600 because the copies can
//! hold secrets.

use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::error::DwError;
use crate::fsutil;
use crate::pending::rfc3339;
use crate::store::Store;

/// Snapshots younger than this are always kept.
pub const RETENTION_DAYS: u64 = 30;
/// Older snapshots survive while they are among this many most recent per file.
pub const MIN_KEEP_PER_FILE: usize = 5;

const META: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapFile {
    /// HOME-relative path.
    pub path: String,
    /// `file` or `symlink`.
    pub kind: String,
    /// Permission bits of a file.
    pub mode: u32,
    pub size: u64,
    /// Name of the blob next to `meta.json` (files only).
    pub blob: Option<String>,
    /// Link target (symlinks only).
    pub link: Option<String>,
    /// SHA-256 of the content (the link target for symlinks).
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    /// UTC, RFC 3339.
    pub created: String,
    pub created_unix: u64,
    /// Why it was taken: `discard`, `restore`, ...
    pub reason: String,
    /// `human` or the agent name.
    pub actor: String,
    pub files: Vec<SnapFile>,
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn private_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    // The umask or an older directory may have left looser permissions.
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn entry_dir(store: &Store, id: &str) -> PathBuf {
    store.dirs.snapshots_dir().join(id)
}

/// Copy the current state of `rels` (HOME-relative; missing files are skipped)
/// into a new snapshot. `None` when there was nothing to preserve.
pub fn take(store: &Store, rels: &[String], reason: &str, actor: &str) -> Result<Option<Snapshot>> {
    take_at(store, rels, reason, actor, now_unix())
}

/// [`take`] with an explicit clock (tests of retention).
pub fn take_at(
    store: &Store,
    rels: &[String],
    reason: &str,
    actor: &str,
    now: u64,
) -> Result<Option<Snapshot>> {
    let home = &store.dirs.home;
    let root = store.dirs.snapshots_dir();
    let mut present = Vec::new();
    for rel in rels {
        if fs::symlink_metadata(home.join(rel)).is_ok_and(|m| !m.is_dir()) {
            present.push(rel);
        }
    }
    if present.is_empty() {
        return Ok(None);
    }
    private_dir(&root)?;
    let compact: String = rfc3339(now)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut n = std::process::id() % 10_000;
    let (id, dir) = loop {
        let id = format!("{compact}-{n:04}");
        let dir = root.join(&id);
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => break (id, dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n = (n + 1) % 10_000,
            Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
        }
    };
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let result = (|| -> Result<Vec<SnapFile>> {
        let mut files = Vec::new();
        for (i, rel) in present.iter().enumerate() {
            let abs = home.join(rel);
            let meta = fs::symlink_metadata(&abs)?;
            if meta.file_type().is_symlink() {
                let link = fs::read_link(&abs)?.to_string_lossy().into_owned();
                files.push(SnapFile {
                    path: (*rel).clone(),
                    kind: "symlink".into(),
                    mode: 0,
                    size: 0,
                    blob: None,
                    sha256: sha(link.as_bytes()),
                    link: Some(link),
                });
            } else {
                let blob = format!("blob-{i}");
                let mut src =
                    fs::File::open(&abs).with_context(|| format!("reading {}", abs.display()))?;
                let mut dst = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(dir.join(&blob))?;
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut src, &mut data)?;
                std::io::Write::write_all(&mut dst, &data)?;
                dst.sync_all()?;
                files.push(SnapFile {
                    path: (*rel).clone(),
                    kind: "file".into(),
                    mode: meta.mode() & 0o7777,
                    size: meta.len(),
                    blob: Some(blob),
                    sha256: sha(&data),
                    link: None,
                });
            }
        }
        Ok(files)
    })();
    let files = match result {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let snap = Snapshot {
        id,
        created: rfc3339(now),
        created_unix: now,
        reason: reason.to_string(),
        actor: actor.to_string(),
        files,
    };
    let written = serde_json::to_vec_pretty(&snap)
        .map_err(anyhow::Error::from)
        .and_then(|b| fsutil::write_atomic(&dir.join(META), b));
    if let Err(e) = written {
        let _ = fs::remove_dir_all(&dir);
        return Err(e);
    }
    fs::set_permissions(dir.join(META), fs::Permissions::from_mode(0o600))?;
    // Retention is housekeeping: failing to prune never fails the overwrite.
    let _ = prune(store, now, RETENTION_DAYS, MIN_KEEP_PER_FILE);
    Ok(Some(snap))
}

/// All snapshots, newest first. Unreadable entries are skipped.
pub fn list(store: &Store) -> Result<Vec<Snapshot>> {
    let root = store.dirs.snapshots_dir();
    let rd = match fs::read_dir(&root) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", root.display())),
    };
    let mut out: Vec<Snapshot> = rd
        .flatten()
        .filter_map(|e| fs::read(e.path().join(META)).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect();
    out.sort_by(|a, b| {
        b.created_unix
            .cmp(&a.created_unix)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(out)
}

/// Snapshots that contain `rel` itself or something below it.
pub fn list_for(store: &Store, rel: &str) -> Result<Vec<Snapshot>> {
    let below = format!("{rel}/");
    Ok(list(store)?
        .into_iter()
        .filter(|s| {
            s.files
                .iter()
                .any(|f| f.path == rel || f.path.starts_with(&below))
        })
        .collect())
}

/// Delete snapshots older than `days` unless they are among the `keep` most
/// recent for one of their files. Returns the removed ids.
pub fn prune(store: &Store, now: u64, days: u64, keep: usize) -> Result<Vec<String>> {
    let all = list(store)?; // newest first
    let cutoff = now.saturating_sub(days * 86_400);
    let mut seen: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut removed = Vec::new();
    for snap in &all {
        let mut keep_it = snap.created_unix >= cutoff;
        for f in &snap.files {
            let n = seen.entry(f.path.as_str()).or_insert(0);
            *n += 1;
            if *n <= keep {
                keep_it = true;
            }
        }
        if !keep_it {
            fs::remove_dir_all(entry_dir(store, &snap.id))?;
            removed.push(snap.id.clone());
        }
    }
    Ok(removed)
}

pub fn find(store: &Store, id: &str) -> Result<Snapshot> {
    if id.is_empty() || id.contains('/') || id.contains("..") {
        return Err(DwError::usage(format!("{id:?} is not a snapshot id")).into());
    }
    let bytes = fs::read(entry_dir(store, id).join(META)).map_err(|_| {
        anyhow::Error::from(DwError::usage(format!(
            "no snapshot {id:?}; see `dw snapshot list`"
        )))
    })?;
    serde_json::from_slice(&bytes).with_context(|| format!("snapshot {id} is unreadable"))
}

/// What a restore does to one file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RestorePlan {
    pub path: String,
    pub kind: String,
    /// Whether HOME currently holds something at the path that will be replaced.
    pub replaces: bool,
}

fn select<'a>(snap: &'a Snapshot, paths: &[String]) -> Result<Vec<&'a SnapFile>> {
    if paths.is_empty() {
        return Ok(snap.files.iter().collect());
    }
    let mut out = Vec::new();
    for p in paths {
        let below = format!("{p}/");
        let hits: Vec<_> = snap
            .files
            .iter()
            .filter(|f| f.path == *p || f.path.starts_with(&below))
            .collect();
        if hits.is_empty() {
            return Err(
                DwError::usage(format!("snapshot {} does not contain ~/{p}", snap.id)).into(),
            );
        }
        out.extend(hits);
    }
    out.dedup_by(|a, b| a.path == b.path);
    Ok(out)
}

pub fn plan_restore(store: &Store, snap: &Snapshot, paths: &[String]) -> Result<Vec<RestorePlan>> {
    Ok(select(snap, paths)?
        .into_iter()
        .map(|f| RestorePlan {
            path: f.path.clone(),
            kind: f.kind.clone(),
            replaces: fs::symlink_metadata(store.dirs.home.join(&f.path)).is_ok(),
        })
        .collect())
}

/// Put the files of snapshot `id` (all, or those under `paths`) back into HOME.
/// What HOME holds now is snapshotted first, so a restore can be undone too.
/// Returns the restored paths and the id of that safety snapshot.
pub fn restore(
    store: &Store,
    id: &str,
    paths: &[String],
    actor: &str,
) -> Result<(Vec<String>, Option<String>)> {
    let snap = find(store, id)?;
    let chosen = select(&snap, paths)?;
    let home = &store.dirs.home;
    for f in &chosen {
        check_destination(home, &f.path)?;
    }
    let rels: Vec<String> = chosen.iter().map(|f| f.path.clone()).collect();
    let safety = take(store, &rels, "restore", actor)?.map(|s| s.id);
    let dir = entry_dir(store, id);
    for f in &chosen {
        let dest = home.join(&f.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        match f.kind.as_str() {
            "symlink" => {
                let link = f.link.as_deref().context("snapshot entry has no link")?;
                let tmp = dest.with_file_name(format!(".dw-restore-{}", std::process::id()));
                let _ = fs::remove_file(&tmp);
                symlink(link, &tmp)?;
                fs::rename(&tmp, &dest).inspect_err(|_| {
                    let _ = fs::remove_file(&tmp);
                })?;
            }
            _ => {
                let blob = f.blob.as_deref().context("snapshot entry has no blob")?;
                let data = fs::read(dir.join(blob))
                    .with_context(|| format!("reading the snapshot copy of ~/{}", f.path))?;
                fsutil::write_atomic(&dest, data)?;
                fs::set_permissions(&dest, fs::Permissions::from_mode(f.mode))?;
            }
        }
    }
    Ok((rels, safety))
}

/// Refuse destinations that are folders or sit below a symlink: a restore must
/// not write through a link that now points elsewhere.
fn check_destination(home: &Path, rel: &str) -> Result<()> {
    let mut cur = home.to_path_buf();
    let comps: Vec<&str> = rel.split('/').collect();
    for (i, c) in comps.iter().enumerate() {
        cur.push(c);
        let Ok(m) = fs::symlink_metadata(&cur) else {
            break;
        };
        let last = i + 1 == comps.len();
        if !last && m.file_type().is_symlink() {
            bail!(
                "~/{rel}: {} is now a symlink; refusing to write through it",
                cur.display()
            );
        }
        if last && m.is_dir() {
            bail!("~/{rel} is now a folder; refusing to replace it");
        }
    }
    Ok(())
}
