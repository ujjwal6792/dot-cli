//! Isolated historical source rendering and guarded HOME writes.
use crate::{changes, store::Store, track};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

pub struct SourceView {
    pub store: Store,
    _temp: tempfile::TempDir,
}
impl SourceView {
    pub fn new(store: &Store, revision: &str) -> Result<Self> {
        let temp = tempfile::Builder::new().prefix("dw-history-").tempdir()?;
        let mut dirs = store.dirs.clone();
        dirs.data = temp.path().join("data");
        dirs.cache = temp.path().join("cache");
        let mut env = store.env().clone();
        env.insert("XDG_DATA_HOME".into(), dirs.data.clone().into());
        env.insert("XDG_CACHE_HOME".into(), dirs.cache.clone().into());
        env.insert("DOTWARDEN_SESSION_BACKEND".into(), "file".into());
        let view = Store::new(dirs, store.tools.clone(), env);
        fs::create_dir_all(view.dirs.data_dir())?;
        let mut cmd = store.git_command(&store.repo());
        cmd.args(["clone", "--quiet", "--shared", "--no-checkout", "--"])
            .arg(store.repo())
            .arg(view.repo());
        crate::exec::run_ok(cmd, None)?;
        view.git(&["checkout", "--quiet", "--detach", revision])?;
        let name = store
            .local_machine_name()?
            .context("register this machine first")?;
        view.write_chezmoi_config(&name)?;
        if let Some(id) = crate::vault::session::identity(store) {
            crate::vault::session::start(&view, &id)?;
        }
        Ok(Self {
            store: view,
            _temp: temp,
        })
    }
    pub(crate) fn targets(&self) -> Result<BTreeMap<String, changes::Target>> {
        if !crate::values::locked_files(&self.store)?.is_empty() {
            return Err(crate::vault::locked());
        }
        changes::targets(&self.store)
    }
}
impl Drop for SourceView {
    fn drop(&mut self) {
        let _ = crate::vault::session::end(&self.store);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeFile {
    pub text: Option<String>,
    pub mode: u32,
}
impl HomeFile {
    pub fn read(store: &Store, path: &str) -> Result<Self> {
        checked_path(store, path)?;
        let abs = store.dirs.home.join(path);
        match fs::symlink_metadata(&abs) {
            Ok(m) if m.is_file() => Ok(Self {
                text: Some(
                    fs::read_to_string(abs).context("only UTF-8 regular files are supported")?,
                ),
                mode: m.permissions().mode() & 0o777,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                text: None,
                mode: 0o644,
            }),
            _ => bail!("~/{path}: only regular files or absent files are supported"),
        }
    }
    pub fn write(&self, store: &Store, path: &str) -> Result<()> {
        checked_path(store, path)?;
        let abs = store.dirs.home.join(path);
        match &self.text {
            Some(text) => {
                crate::fsutil::write_atomic(&abs, text)?;
                fs::set_permissions(abs, fs::Permissions::from_mode(self.mode))?;
            }
            None => match fs::remove_file(abs) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            },
        }
        Ok(())
    }
}
pub fn checked_path(store: &Store, path: &str) -> Result<()> {
    use std::path::Component;
    let p = std::path::Path::new(path);
    if p.as_os_str().is_empty() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("invalid HOME-relative path");
    }
    let mut abs = store.dirs.home.clone();
    for part in p.components() {
        abs.push(part);
        if fs::symlink_metadata(&abs).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("~/{path}: symlinks are unsupported for this operation");
        }
    }
    Ok(())
}
pub fn fingerprint(store: &Store) -> Result<String> {
    Ok(format!(
        "{}:{}:{}",
        crate::history::resolve(store, "HEAD")?,
        changes::sha256(
            store
                .git(&[
                    "--no-optional-locks",
                    "diff",
                    "--cached",
                    "--binary",
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD",
                    "--"
                ])?
                .stdout
                .as_bytes()
        ),
        store.local_machine_name()?.unwrap_or_default()
    ))
}

/// Preview guard for Git UI commands: refs, index and tracked HOME state.
pub fn preview_guard(store: &Store) -> Result<String> {
    let mut text = fingerprint(store)?;
    text.push_str(&serde_json::to_string(&crate::branches::list(store)?)?);
    text.push_str(&serde_json::to_string(&crate::stashes::list(store)?)?);
    for p in crate::source::load(&store.repo())?.files.keys() {
        text.push_str(p);
        text.push_str(&track::home_fingerprint(&store.dirs.home.join(p)));
    }
    Ok(changes::sha256(text.as_bytes()))
}
pub fn safe_plain(store: &Store, text: &str) -> Result<()> {
    let known = crate::detect::known_pairs(store);
    if !crate::detect::scan_text(text).is_empty()
        || known.iter().any(|(_, v)| !v.is_empty() && text.contains(v))
    {
        bail!(
            "secret-like content needs an initialized, unlocked vault before it can be snapshotted or stashed"
        );
    }
    Ok(())
}
pub fn safety_snapshot(store: &Store, paths: &[String], reason: &str) -> Result<Option<String>> {
    if crate::vault::exists(store) {
        crate::vault::session::require(store)?;
        Ok(crate::snapshot::take_protected(
            store,
            paths,
            reason,
            &crate::pending::requester(store),
        )?
        .map(|s| s.id))
    } else {
        for p in paths {
            if let Some(t) = HomeFile::read(store, p)?.text {
                safe_plain(store, &t)?;
            }
        }
        Ok(
            crate::snapshot::take(store, paths, reason, &crate::pending::requester(store))?
                .map(|s| s.id),
        )
    }
}

pub fn supported_templates(store: &Store, paths: &[String]) -> Result<()> {
    let tracked = crate::source::load(&store.repo())?;
    let variants = crate::variants::Variants::read(&store.repo())?;
    for path in paths {
        let Some(entry) = tracked.files.get(path) else {
            continue;
        };
        let text = fs::read_to_string(store.repo().join(&entry.source))?;
        if entry.source.ends_with(".tmpl")
            && crate::values::parse(&text).is_none()
            && crate::render::file_id(&text).is_none()
            && text != crate::variants::template(path)
        {
            bail!(
                "~/{path}: custom templates are unsupported; only Dotwarden references/variants/encrypted files can be preserved"
            );
        }
        for file in variants.files_of(&store.repo(), path) {
            let text = fs::read_to_string(store.repo().join(file))?;
            if text.contains("{{") && crate::values::parse(&text).is_none() {
                bail!("~/{path}: custom variant templates are unsupported");
            }
        }
    }
    Ok(())
}
