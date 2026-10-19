use crate::{
    backend,
    config::{Config, atomic_write, clean_relative},
    operations::Lock,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    pub entries: Vec<(String, String)>,
    pub held: Vec<String>,
    pub destination: PathBuf,
}
/// The old catalog is data, never an executable import adapter.
pub fn preview(old: &Path, destination: &Path) -> Result<Migration> {
    let contract_paths = [
        old.join("dot.json"),
        old.join(".dot.json"),
        old.join(".dot/contract.json"),
        old.join("dot.contract.json"),
        old.join("dot-contract.json"),
    ];
    let contract = contract_paths.iter().find(|p| p.exists());
    let (manifest, base) = if let Some(p) = contract {
        let v: serde_json::Value = serde_json::from_slice(&fs::read(p)?)?;
        let manifest = v["manifest"].as_str().unwrap_or("manifest.json");
        let home = v["home_source"].as_str().unwrap_or("home");
        clean_relative(Path::new(manifest))?;
        clean_relative(Path::new(home))?;
        (old.join(manifest), old.join(home))
    } else {
        (old.join("manifest.json"), old.join("home"))
    };
    let v: serde_json::Value = serde_json::from_slice(
        &fs::read(&manifest)
            .with_context(|| format!("Cannot find old manifest at {}", manifest.display()))?,
    )?;
    let mut out = Migration {
        entries: vec![],
        held: vec![],
        destination: destination.into(),
    };
    for (id, e) in v["entries"]
        .as_object()
        .context("Manifest entries missing")?
    {
        let source = e["source"].as_str().unwrap_or("");
        let dest = e["destination"].as_str().unwrap_or("");
        let rel = dest.strip_prefix("~/").unwrap_or(dest);
        let scoped = e["os"].as_array().is_some_and(|a| {
            !(a.is_empty()
                || a.iter().any(|v| v == "all")
                || (a.iter().any(|v| v == "darwin") && a.iter().any(|v| v == "linux")))
        });
        let complex = [
            "template",
            "git_keys",
            "git_values",
            "removals",
            "hooks",
            "integrations",
            "logical_id",
        ]
        .iter()
        .any(|k| {
            e.get(*k).is_some_and(|v| {
                v != &serde_json::Value::Null
                    && v != &serde_json::json!("")
                    && v != &serde_json::json!([])
                    && v != &serde_json::json!({})
            })
        });
        if clean_relative(Path::new(source)).is_err()
            || clean_relative(Path::new(rel)).is_err()
            || scoped
            || complex
            || e.get("machines")
                .is_some_and(|m| m != &serde_json::json!({"include":["all"],"exclude":[]}))
        {
            out.held.push(format!(
                "{id}: scope/template/metadata requires explicit translation"
            ));
            continue;
        }
        let src = base.join(source);
        if src.exists() && !src.canonicalize()?.starts_with(old.canonicalize()?) {
            out.held
                .push(format!("{id}: source resolves outside old catalog"));
            continue;
        }
        if !src.is_file() || fs::symlink_metadata(&src)?.file_type().is_symlink() {
            out.held.push(format!("{id}: source is not a plain file"));
            continue;
        }
        out.entries
            .push((src.to_string_lossy().into_owned(), rel.into()));
    }
    Ok(out)
}
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for e in walkdir::WalkDir::new(from).follow_links(false) {
        let e = e?;
        let dst = to.join(e.path().strip_prefix(from)?);
        if e.file_type().is_dir() {
            fs::create_dir_all(&dst)?;
        } else {
            fs::create_dir_all(dst.parent().unwrap())?;
            if e.file_type().is_symlink() {
                symlink(fs::read_link(e.path())?, &dst)?;
            } else {
                fs::copy(e.path(), &dst)?;
            }
        }
    }
    Ok(())
}
pub fn convert(c: &Config, old: &Path, destination: &Path) -> Result<Migration> {
    let _lock = Lock::acquire(c)?;
    let report = preview(old, destination)?;
    if c.state.starts_with(old) {
        bail!("State/archive directory cannot be inside the old source");
    }
    if destination.exists() {
        bail!("Migration destination must be a new directory");
    }
    if destination.starts_with(old) || old.starts_with(destination) {
        bail!("Old source and destination must be separate");
    }
    let archive = c
        .state
        .join("archives")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&archive)?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o700))?;
    copy_tree(old, &archive.join("source"))?;
    // Preserve local state outside the archive subtree, including old enrollment.
    for e in fs::read_dir(&c.state)? {
        let e = e?;
        if matches!(e.file_name().to_str(), Some("archives" | "operation.lock")) {
            continue;
        }
        let dst = archive.join("local-state").join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dst)?;
        } else if e.file_type()?.is_file() {
            fs::create_dir_all(dst.parent().unwrap())?;
            fs::copy(e.path(), dst)?;
        }
    }
    if let Some(path) = &c.runtime_config
        && path.is_file()
    {
        fs::copy(path, archive.join("dot-config.toml"))?;
    }
    atomic_write(
        &archive.join("dot-config.json"),
        &serde_json::to_vec_pretty(c)?,
    )?;
    // Materialize only convertible source bytes into an isolated destination HOME.
    let temp = tempfile::tempdir()?;
    let mut isolated = c.clone();
    isolated.source = destination.into();
    isolated.home = temp.path().join("home");
    isolated.state = temp.path().join("state");
    let empty = temp.path().join("config/chezmoi.toml");
    fs::create_dir_all(empty.parent().unwrap())?;
    fs::write(&empty, "")?;
    isolated.chezmoi_config = Some(empty);
    fs::create_dir_all(destination)?;
    fs::create_dir_all(&isolated.home)?;
    for (src, rel) in &report.entries {
        let dst = isolated.target(rel)?;
        fs::create_dir_all(dst.parent().unwrap())?;
        fs::copy(src, &dst)?;
        backend::chez(
            &isolated,
            &[
                "add",
                "--recursive=false",
                "--secrets=error",
                "--",
                &dst.to_string_lossy(),
            ],
        )?;
    }
    atomic_write(
        &archive.join("migration.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
