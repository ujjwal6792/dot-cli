//! Offline archive (spec §11): a git bundle of the repository plus what a
//! fresh machine needs to take its place (its registry name and the remote
//! settings), sealed with the vault passphrase.
//!
//! Plain layout before sealing: `MAGIC`, a big-endian `u32` header length, the
//! JSON [`Header`], then the bundle. The seal is age's passphrase encryption,
//! the same as the vault's own, so one passphrase unlocks both.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::track::refuse;
use crate::{fsutil, repo, vault};

const MAGIC: &[u8] = b"DWARCHIVE1\n";
const FORMAT: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    format: u32,
    created: String,
    /// Branch HEAD was on.
    branch: String,
    head: String,
    /// This machine's registry name.
    machine: Option<String>,
    /// `remote.*` and `branch.*` settings of the repository (URLs, upstreams).
    config: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
pub struct Exported {
    pub file: String,
    pub bytes: Option<u64>,
    pub branch: String,
    pub head: String,
    pub machine: Option<String>,
    pub registry: Vec<String>,
    /// Staged changes that are not in the archive: only commits are.
    pub uncommitted: usize,
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct Imported {
    pub branch: String,
    pub head: String,
    pub machine: Option<String>,
    /// Whether the machine was registered again under its old name.
    pub registered: bool,
    pub registry: Vec<String>,
    pub dry_run: bool,
}

fn setting_wanted(key: &str) -> bool {
    let (section, rest) = key.split_once('.').unwrap_or_default();
    let name = key.rsplit('.').next().unwrap_or_default();
    match section {
        "remote" => matches!(name, "url" | "pushurl" | "fetch") && rest.contains('.'),
        "branch" => matches!(name, "remote" | "merge") && rest.contains('.'),
        _ => false,
    }
}

fn seal(store: &Store, pass: &str, header: &Header, bundle: &[u8]) -> Result<Vec<u8>> {
    let head = serde_json::to_vec(header)?;
    let mut plain = Vec::with_capacity(MAGIC.len() + 4 + head.len() + bundle.len());
    plain.extend_from_slice(MAGIC);
    plain.extend_from_slice(&u32::try_from(head.len())?.to_be_bytes());
    plain.extend_from_slice(&head);
    plain.extend_from_slice(bundle);
    vault::wrap(store, pass, &plain)
}

fn unseal(pass: &str, sealed: &[u8]) -> Result<(Header, Vec<u8>)> {
    let plain = vault::unwrap(pass, sealed, "archive passphrase")
        .map_err(|_| refuse("wrong passphrase, or this is not a dotwarden archive".into()))?;
    let body = plain
        .strip_prefix(MAGIC)
        .context("not a dotwarden archive")?;
    let (len, rest) = body.split_first_chunk::<4>().context("truncated archive")?;
    let len = u32::from_be_bytes(*len) as usize;
    if rest.len() < len {
        bail!("truncated archive");
    }
    let header: Header =
        serde_json::from_slice(&rest[..len]).context("unreadable archive header")?;
    if header.format != FORMAT {
        bail!(
            "archive format {} is not supported (this dotwarden reads {FORMAT})",
            header.format
        );
    }
    Ok((header, rest[len..].to_vec()))
}

/// Write the archive of the repository to `dest`. `pass` must be the vault
/// passphrase, so an archive can always be opened with the vault's own.
pub fn export(
    store: &Store,
    pass: &str,
    dest: &Path,
    force: bool,
    dry_run: bool,
) -> Result<Exported> {
    if !vault::exists(store) {
        return Err(refuse(
            "the archive is sealed with the vault passphrase, and there is no vault yet; run `dw vault init`"
                .into(),
        ));
    }
    if dest.exists() && !force {
        return Err(refuse(format!(
            "{} exists; pass --force to replace it",
            dest.display()
        )));
    }
    let _lock = if dry_run { None } else { Some(store.lock()?) };
    // The passphrase must be the vault's: that is what makes the archive recoverable.
    vault::unwrap(
        pass,
        &vault::read_wrapped(store, "identity.age")?,
        "passphrase",
    )?;
    let branch = store
        .git(&["symbolic-ref", "--short", "HEAD"])
        .map(|o| o.stdout.trim().to_string())
        .map_err(|_| refuse("the repository is not on a branch".into()))?;
    let head = store.git(&["rev-parse", "HEAD"])?.stdout.trim().to_string();
    let registry = repo::read_registry(&store.repo())?
        .into_iter()
        .map(|m| m.name)
        .collect();
    let uncommitted = store
        .git(&["diff", "--cached", "--name-only"])?
        .stdout
        .lines()
        .count();
    let out = Exported {
        file: dest.display().to_string(),
        bytes: None,
        branch: branch.clone(),
        head: head.clone(),
        machine: store.local_machine_name()?,
        registry,
        uncommitted,
        dry_run,
    };
    if dry_run {
        return Ok(out);
    }
    let config = store
        .git(&[
            "config",
            "--local",
            "--null",
            "--get-regexp",
            r"^(remote|branch)\.",
        ])
        .map(|o| o.stdout)
        .unwrap_or_default()
        .split('\0')
        .filter_map(|l| l.split_once('\n'))
        .filter(|(k, _)| setting_wanted(k))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let header = Header {
        format: FORMAT,
        created: crate::pending::now_rfc3339(),
        branch,
        head,
        machine: out.machine.clone(),
        config,
    };
    let tmp = tempfile::Builder::new()
        .prefix("dw-bundle-")
        .tempfile_in(store.dirs.state_dir())
        .or_else(|_| tempfile::NamedTempFile::new())?;
    let path = tmp.path().to_string_lossy().into_owned();
    store.git(&["bundle", "create", "--quiet", &path, "--all"])?;
    let sealed = seal(store, pass, &header, &fs::read(tmp.path())?)?;
    fsutil::write_atomic(dest, &sealed)?;
    fs::set_permissions(dest, fs::Permissions::from_mode(0o600))?;
    Ok(Exported {
        bytes: Some(sealed.len() as u64),
        ..out
    })
}

/// Restore a fresh machine from the archive in `file`: the repository with
/// every branch, its remotes, and the machine's registry name. Needs no network.
pub fn import(store: &Store, pass: &str, file: &Path, dry_run: bool) -> Result<Imported> {
    let sealed = fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let (header, bundle) = unseal(pass, &sealed)?;
    repo::require_empty_repo_location(store)?;
    let mut out = Imported {
        branch: header.branch.clone(),
        head: header.head.clone(),
        machine: header.machine.clone(),
        registered: false,
        registry: Vec::new(),
        dry_run,
    };
    if dry_run {
        out.registered = header.machine.is_some();
        return Ok(out);
    }
    repo::require_git_identity(store)?;
    let _lock = store.lock()?;
    store.ensure_dirs()?;
    let repo_dir = store.repo();
    repo::cleaning_up_on_error(store, || {
        fs::create_dir_all(&repo_dir)?;
        let mut tmp = tempfile::Builder::new()
            .prefix("dw-bundle-")
            .tempfile_in(store.dirs.state_dir())?;
        std::io::Write::write_all(&mut tmp, &bundle)?;
        let path = tmp.path().to_string_lossy().into_owned();
        store.git(&["init", "--quiet"])?;
        store.git(&[
            "fetch",
            "--quiet",
            "--update-head-ok",
            &path,
            "+refs/*:refs/*",
        ])?;
        let branch_ref = format!("refs/heads/{}", header.branch);
        store.git(&["symbolic-ref", "HEAD", &branch_ref])?;
        store.git(&["reset", "--hard", "--quiet"])?;
        for (key, value) in header.config.iter().filter(|(k, _)| setting_wanted(k)) {
            store.git(&["config", "--add", key, value])?;
        }
        repo::check_repository(store, &file.display().to_string())
    })?;
    out.registry = repo::read_registry(&store.repo())?
        .into_iter()
        .map(|m| m.name)
        .collect();
    if let Some(name) = &header.machine
        && out.registry.contains(name)
    {
        store.write_chezmoi_config(name)?;
        store.write_local_machine(name)?;
        out.registered = true;
    }
    Ok(out)
}
