//! Creating and cloning the Dotwarden repository, and the machine registry
//! (spec §3, §4).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::hide::DEFAULT_RULES;
use crate::machine::{self, Machine};
use crate::store::{FORMAT_VERSION, Store};
use crate::templates;

/// Default branch of new repositories.
pub const DEFAULT_BRANCH: &str = "main";

/// Registry and other Dotwarden data, relative to the repository root.
pub const DATA_FILE: &str = "home/.chezmoidata/dotwarden.toml";
/// One file per machine, so registrations made on different computers never
/// conflict textually; chezmoi deep-merges them into `dotwarden.machines`.
pub const MACHINES_DIR: &str = "home/.chezmoidata/dotwarden-machines";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub machine: Machine,
    /// False when an existing registry entry was adopted.
    pub created: bool,
}

/// Repository files `dw init` creates, relative to the repository root.
pub fn layout_paths() -> Vec<String> {
    let mut v: Vec<String> = [".chezmoiroot", "dotwarden.toml", "hide", DATA_FILE]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(templates::FILES.iter().map(|(rel, _)| rel.to_string()));
    v
}

fn write(root: &Path, rel: &str, content: &str) -> Result<()> {
    let path = root.join(rel);
    crate::fsutil::write_atomic(&path, content)
}

/// Write the §3 repository layout into `root` (which must exist).
pub fn write_layout(root: &Path, branch: &str) -> Result<()> {
    write(root, ".chezmoiroot", "home\n")?;
    write(
        root,
        "dotwarden.toml",
        &format!("format = {FORMAT_VERSION}\ndefault_branch = \"{branch}\"\n"),
    )?;
    write(root, "hide", DEFAULT_RULES)?;
    for (rel, content) in templates::FILES {
        write(root, rel, content)?;
    }
    write(
        root,
        DATA_FILE,
        &format!(
            "# Assignments, variants and values (see docs/spec.md §4). The machine registry\n\
             # lives in dotwarden-machines/, one file per machine.\n\
             # Edited by dotwarden and by hand; comments and ordering are preserved.\n\
             [dotwarden]\nformat = {FORMAT_VERSION}\n"
        ),
    )
}

pub fn require_empty_repo_location(store: &Store) -> Result<()> {
    let repo = store.repo();
    let occupied = repo.exists()
        && (!repo.is_dir()
            || fs::read_dir(&repo)
                .map(|mut d| d.next().is_some())
                .unwrap_or(true));
    if occupied {
        bail!(
            "a repository already exists at {} (remove it to start over)",
            repo.display()
        );
    }
    Ok(())
}

pub fn require_git_identity(store: &Store) -> Result<()> {
    if store.git_identity_value("user.name").is_none()
        || store.git_identity_value("user.email").is_none()
    {
        bail!(
            "git identity is not configured; run `git config --global user.name \"Your Name\"` \
             and `git config --global user.email you@example.com` (dotwarden reads ~/.gitconfig)"
        );
    }
    Ok(())
}

pub fn commit_all(store: &Store, message: &str) -> Result<()> {
    store.git(&["add", "-A"])?;
    store.git(&["commit", "--quiet", "-m", message])?;
    Ok(())
}

/// Run `body`; on failure remove the half-created repository so the command
/// can simply be retried.
fn cleaning_up_on_error<T>(store: &Store, body: impl FnOnce() -> Result<T>) -> Result<T> {
    let result = body();
    if result.is_err() {
        let _ = fs::remove_dir_all(store.repo());
    }
    result
}

/// `dw init`: create a new repository containing the layout, the fixed
/// templates and default hide rules, with no machine registered yet.
pub fn init(store: &Store, branch: &str) -> Result<()> {
    require_empty_repo_location(store)?;
    require_git_identity(store)?;
    store.ensure_dirs()?;
    let repo = store.repo();
    fs::create_dir_all(&repo)?;
    cleaning_up_on_error(store, || {
        store.git(&["init", "--quiet", "--initial-branch", branch])?;
        write_layout(&repo, branch)?;
        commit_all(store, "Initialise Dotwarden repository")
    })
}

/// `dw clone`: clone `url` into the store and check it is a Dotwarden
/// repository. No machine is registered yet.
pub fn clone(store: &Store, url: &str) -> Result<()> {
    require_empty_repo_location(store)?;
    require_git_identity(store)?;
    store.ensure_dirs()?;
    let repo = store.repo();
    cleaning_up_on_error(store, || {
        let mut cmd = store.git_command(&store.dirs.data_dir());
        cmd.args(["clone", "--quiet", "--"]).arg(url).arg(&repo);
        crate::exec::run_ok(cmd, None)?;
        if !store.is_initialised() {
            bail!("{url} is not a Dotwarden repository (missing .chezmoiroot or dotwarden.toml)");
        }
        let text = fs::read_to_string(repo.join("dotwarden.toml"))?;
        let t: toml::Table = toml::from_str(&text).context("parsing dotwarden.toml")?;
        match t.get("format").and_then(toml::Value::as_integer) {
            Some(f) if f == i64::from(FORMAT_VERSION) => Ok(()),
            other => bail!(
                "unsupported repository format {other:?} (this dotwarden understands {FORMAT_VERSION})"
            ),
        }
    })
}

// ---- machine registry -------------------------------------------------

/// Read the machine registry (one file per machine), sorted by name.
pub fn read_registry(repo: &Path) -> Result<Vec<Machine>> {
    let dir = repo.join(MACHINES_DIR);
    let mut out = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "toml") {
            continue;
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let doc: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let table = doc
            .get("dotwarden")
            .and_then(|d| d.get("machines"))
            .and_then(toml::Value::as_table)
            .with_context(|| {
                format!(
                    "{} has no [dotwarden.machines.<name>] table",
                    path.display()
                )
            })?;
        for (_, v) in table {
            out.push(
                v.clone()
                    .try_into::<Machine>()
                    .with_context(|| format!("parsing the machine in {}", path.display()))?,
            );
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Edit the data file as a TOML document: comments, ordering and formatting
/// of everything not touched survive. The write is atomic.
pub fn edit_data(
    repo: &Path,
    edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>,
) -> Result<()> {
    let path = repo.join(DATA_FILE);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    edit(&mut doc)?;
    crate::fsutil::write_atomic(&path, doc.to_string())
}

/// The `[dotwarden]` table of the data file, created if missing.
pub fn dotwarden_table(doc: &mut toml_edit::DocumentMut) -> Result<&mut toml_edit::Table> {
    doc.entry("dotwarden")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("`dotwarden` in the data file is not a table")
}

fn machine_file(m: &Machine) -> Result<String> {
    let mut t = toml::Table::new();
    t.insert("name".into(), m.name.clone().into());
    t.insert("platform".into(), m.platform.clone().into());
    t.insert("distribution".into(), m.distribution.clone().into());
    t.insert(
        "families".into(),
        toml::Value::Array(
            m.families
                .iter()
                .cloned()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    t.insert("retired".into(), m.retired.into());
    let mut machines = toml::Table::new();
    machines.insert(m.name.clone(), toml::Value::Table(t));
    let mut dw = toml::Table::new();
    dw.insert("machines".into(), toml::Value::Table(machines));
    let mut root = toml::Table::new();
    root.insert("dotwarden".into(), toml::Value::Table(dw));
    Ok(toml::to_string(&root)?)
}

/// Make the registry files match `machines`: one file per machine, rewritten only
/// when its text changes, files of machines no longer listed removed.
pub fn write_registry(repo: &Path, machines: &[Machine]) -> Result<()> {
    let dir = repo.join(MACHINES_DIR);
    fs::create_dir_all(&dir)?;
    let mut keep = Vec::new();
    for m in machines {
        machine::validate_name(&m.name)?;
        let file = dir.join(format!("{}.toml", m.name));
        let text = machine_file(m)?;
        if fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            crate::fsutil::write_atomic(&file, text)?;
        }
        keep.push(file);
    }
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|x| x == "toml") && !keep.contains(&path) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Register this computer under `name` (spec §4) and commit the registry.
/// With `adopt`, an existing entry for the same platform and distribution is
/// reused (reinstalling a machine) instead of being an error.
pub fn register_machine(store: &Store, name: &str, adopt: bool) -> Result<Registration> {
    machine::validate_name(name)?;
    let detected = machine::detect()?;
    register_detected(store, name, detected, adopt)
}

pub fn register_detected(
    store: &Store,
    name: &str,
    detected: machine::Detected,
    adopt: bool,
) -> Result<Registration> {
    machine::validate_name(name)?;
    require_git_identity(store)?;
    let repo = store.repo();
    let mut registry = read_registry(&repo)?;
    let fresh = detected.into_machine(name);

    let registration = match registry.iter().position(|m| m.name == name) {
        None => {
            registry.push(fresh.clone());
            write_registry(&repo, &registry)?;
            commit_all(store, &format!("Register machine {name}"))?;
            Registration {
                machine: fresh,
                created: true,
            }
        }
        Some(i) => {
            let existing = &registry[i];
            if existing.retired {
                bail!("machine name {name:?} belongs to a retired machine; choose another name");
            }
            if !adopt {
                bail!(
                    "machine name {name:?} is already registered ({}/{}); choose another name, \
                     or pass --adopt if this computer is that machine again",
                    existing.platform,
                    existing.distribution
                );
            }
            if existing.platform != fresh.platform || existing.distribution != fresh.distribution {
                bail!(
                    "cannot adopt {name:?}: it is registered as {}/{} but this computer is {}/{}",
                    existing.platform,
                    existing.distribution,
                    fresh.platform,
                    fresh.distribution
                );
            }
            if existing.families != fresh.families {
                registry[i].families = fresh.families.clone();
                write_registry(&repo, &registry)?;
                commit_all(store, &format!("Update machine {name}"))?;
            }
            Registration {
                machine: registry[i].clone(),
                created: false,
            }
        }
    };
    store.write_local_machine(name)?;
    store.write_chezmoi_config(name)?;
    Ok(registration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roundtrip_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        write_layout(dir.path(), "main").unwrap();
        let extra = dir.path().join(DATA_FILE);
        let mut text = fs::read_to_string(&extra).unwrap();
        text.push_str("\n[dotwarden.assignments.\".zshrc\"]\ninclude = [\"platform:linux\"]\n");
        fs::write(&extra, text).unwrap();

        let m = Machine {
            name: "box".into(),
            platform: "linux".into(),
            distribution: "ubuntu".into(),
            families: vec!["debian".into()],
            retired: false,
        };
        write_registry(dir.path(), std::slice::from_ref(&m)).unwrap();
        assert_eq!(read_registry(dir.path()).unwrap(), vec![m]);
        let data: toml::Table =
            toml::from_str(&fs::read_to_string(dir.path().join(DATA_FILE)).unwrap()).unwrap();
        assert!(data["dotwarden"]["assignments"][".zshrc"]["include"].is_array());
    }

    #[test]
    fn layout_contains_everything_in_the_spec() {
        let dir = tempfile::tempdir().unwrap();
        write_layout(dir.path(), "main").unwrap();
        for rel in [
            ".chezmoiroot",
            "dotwarden.toml",
            "hide",
            "home/.chezmoiignore",
            "home/.chezmoidata/dotwarden.toml",
            "home/.chezmoitemplates/dotwarden/targets",
            "home/.chezmoitemplates/dotwarden/variant",
        ] {
            assert!(dir.path().join(rel).is_file(), "{rel}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(".chezmoiroot")).unwrap(),
            "home\n"
        );
    }
}
