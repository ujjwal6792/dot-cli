use crate::{
    config::{Config, clean_relative},
    git::{self, GitState},
    ignores, process,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Managed {
    pub absolute: String,
    pub source_absolute: String,
    pub source_relative: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub managed: BTreeMap<String, Managed>,
    #[serde(default)]
    pub managed_directories: BTreeSet<String>,
    /// Native-approved immediate entries, inspected in background workers only.
    #[serde(default)]
    pub directory_entries: BTreeMap<String, BTreeSet<String>>,
    pub changes: BTreeMap<String, String>,
    pub git: GitState,
    pub observed_at: String,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub managed: bool,
    pub changed: bool,
    pub children: usize,
    pub status: String,
}

/// A valid selection with no eligible targets, rather than an inspection failure.
#[derive(Debug)]
pub struct NoEligibleFiles;

impl std::fmt::Display for NoEligibleFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("No eligible files in selection. Ignored files are skipped; check .chezmoiignore and dot’s default exclusions")
    }
}

impl std::error::Error for NoEligibleFiles {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Managed,
    Changed,
}
impl Filter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Managed,
            Self::Managed => Self::Changed,
            Self::Changed => Self::All,
        }
    }
}

pub fn chez(c: &Config, args: &[&str]) -> Result<String> {
    chez_args(c, process::strings(args))
}
pub fn chez_args(c: &Config, args: Vec<String>) -> Result<String> {
    let mut base = process::strings(&[
        "--source",
        &c.source.to_string_lossy(),
        "--destination",
        &c.home.to_string_lossy(),
        "--persistent-state",
        &c.state.join("chezmoi.db").to_string_lossy(),
        "--cache",
        &c.state.join("cache").to_string_lossy(),
        "--no-tty",
        "--no-pager",
        "--color=false",
        "--use-builtin-diff",
    ]);
    if let Some(p) = &c.chezmoi_config {
        base.extend(["--config".into(), p.to_string_lossy().into_owned()]);
    }
    base.extend(args);
    process::run(&c.chezmoi, &base, None, None)
}
pub fn snapshot(c: &Config) -> Result<Snapshot> {
    c.validate()?;
    if !c.source.exists() {
        bail!("Source does not exist. Use dot init to connect a chezmoi repository.");
    }
    let managed: BTreeMap<String, Managed> = serde_json::from_str(&chez(
        c,
        &[
            "managed",
            "--format=json",
            "--path-style=all",
            "--include=files,symlinks",
        ],
    )?)
    .context("Invalid chezmoi managed JSON")?;
    for (path, item) in &managed {
        clean_relative(Path::new(path))?;
        if !Path::new(&item.source_absolute).starts_with(&c.source) {
            bail!("Managed source outside configured repository: {path}");
        }
    }
    let managed_directories = chez(
        c,
        &["managed", "--path-style=relative", "--include=dirs", "-0"],
    )?
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_owned)
    .collect();
    let changes = parse_status(&chez(c, &["status", "--path-style=relative"])?);
    let git = git::status(c).unwrap_or_else(|e| GitState {
        error: Some(e.to_string()),
        ..Default::default()
    });
    let mut snapshot = Snapshot {
        managed,
        managed_directories,
        directory_entries: BTreeMap::new(),
        changes,
        git,
        observed_at: chrono::Utc::now().to_rfc3339(),
        warnings: vec![],
    };
    inspect_directory(c, &mut snapshot, "")?;
    Ok(snapshot)
}
pub fn parse_status(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| l.len() >= 4 && l.is_char_boundary(3))
        .map(|l| (l[3..].to_string(), l[..2].to_string()))
        .collect()
}
/// Inspect only the current directory, never recursively scan HOME. Native
/// unmanaged prunes each unmanaged directory immediately. Managed directories
/// must be excluded from its arguments because chezmoi would descend into them.
/// Call from a worker; rows() itself only reads this cache and the local disk.
pub fn inspect_directory(c: &Config, s: &mut Snapshot, directory: &str) -> Result<()> {
    let base = if directory.is_empty() {
        c.home.clone()
    } else {
        c.target(directory)?
    };
    // A user can navigate during the initial inspection. Check the directory
    // and its ancestors as well, so a directly queried child cannot bypass an
    // ignored parent that native tree walking would have pruned.
    let mut ancestors = Vec::new();
    let mut ancestor = Path::new(directory);
    while !ancestor.as_os_str().is_empty() {
        let relative = ancestor.to_string_lossy().into_owned();
        if !s.managed_directories.contains(&relative) && c.home.join(ancestor).is_dir() {
            ancestors.push(relative);
        }
        ancestor = ancestor.parent().unwrap_or_else(|| Path::new(""));
    }
    if !ancestors.is_empty() && native_unmanaged(c, &ancestors)?.len() != ancestors.len() {
        s.directory_entries
            .insert(directory.to_owned(), BTreeSet::new());
        return Ok(());
    }
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    let mut candidates = Vec::new();
    let mut visible = BTreeSet::new();
    if base.is_dir() {
        for entry in fs::read_dir(&base)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = format!("{prefix}{name}");
            if entry.path().starts_with(&c.source)
                || entry.path().starts_with(&c.state)
                || name == ".git"
            {
                continue;
            }
            if s.managed.contains_key(&path) || s.managed_directories.contains(&path) {
                visible.insert(path);
            } else {
                candidates.push(path);
            }
        }
    }
    visible.extend(native_unmanaged(c, &candidates)?);
    s.directory_entries.insert(directory.to_owned(), visible);
    Ok(())
}

fn native_unmanaged(c: &Config, candidates: &[String]) -> Result<BTreeSet<String>> {
    let mut visible = BTreeSet::new();
    for chunk in candidates.chunks(128) {
        let mut args = process::strings(&[
            "unmanaged",
            "--path-style=relative",
            "-0",
            "--include=files,symlinks,dirs",
            "--",
        ]);
        for path in chunk {
            args.push(c.target(path)?.to_string_lossy().into_owned());
        }
        let output = chez_args(c, args)?;
        // Only accept requested immediate entries. This also prevents unexpected
        // native output from ever broadening the scope of the browsing cache.
        let accepted: BTreeSet<_> = output.split('\0').collect();
        visible.extend(
            chunk
                .iter()
                .filter(|p| accepted.contains(p.as_str()))
                .cloned(),
        );
    }
    Ok(visible)
}

pub fn rows(
    c: &Config,
    s: &Snapshot,
    directory: &str,
    filter: Filter,
    query: &str,
) -> Result<Vec<Row>> {
    let base = if directory.is_empty() {
        c.home.clone()
    } else {
        c.target(directory)?
    };
    let mut names: BTreeSet<String> = BTreeSet::new();
    if base.is_dir() {
        for entry in fs::read_dir(&base)? {
            let e = entry?;
            if let Some(n) = e.file_name().to_str() {
                names.insert(n.to_string());
            }
        }
    }
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    for path in s.managed.keys() {
        if let Some(rel) = path.strip_prefix(&prefix)
            && let Some(n) = rel.split('/').next()
        {
            names.insert(n.to_string());
        }
    }
    let git_changed: BTreeSet<_> = s.git.files.iter().map(|f| f.path.as_str()).collect();
    let mut rows = vec![];
    for name in names {
        let path = format!("{prefix}{name}");
        let inspected = s.directory_entries.get(directory);
        if (inspected.is_some_and(|visible| !visible.contains(&path))
            || inspected.is_none() && !s.observed_at.is_empty())
            && !s.managed.contains_key(&path)
            && !s.managed_directories.contains(&path)
            && !s.managed.keys().any(|p| p.starts_with(&format!("{path}/")))
        {
            continue;
        }
        let child_prefix = format!("{path}/");
        let descendants: Vec<_> = s
            .managed
            .iter()
            .filter(|(p, _)| p.starts_with(&child_prefix))
            .collect();
        let directory = !descendants.is_empty()
            || fs::symlink_metadata(c.home.join(&path)).is_ok_and(|m| m.is_dir());
        let managed = s.managed.contains_key(&path) || !descendants.is_empty();
        let changed = s
            .changes
            .keys()
            .any(|p| p == &path || p.starts_with(&child_prefix))
            || s.managed.iter().any(|(p, m)| {
                (p == &path || p.starts_with(&child_prefix))
                    && git_changed.contains(m.source_relative.as_str())
            });
        if filter == Filter::Managed && !managed
            || filter == Filter::Changed && !changed
            || !name.to_lowercase().contains(&query.to_lowercase())
        {
            continue;
        }
        let status = s.changes.get(&path).cloned().unwrap_or_else(|| {
            if changed {
                "M"
            } else if managed {
                "="
            } else {
                "?"
            }
            .into()
        });
        rows.push(Row {
            path,
            name,
            directory,
            managed,
            changed,
            children: descendants.len(),
            status,
        });
    }
    rows.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    Ok(rows)
}
pub fn exact_targets(
    c: &Config,
    s: &Snapshot,
    paths: &[String],
    capture: bool,
) -> Result<Vec<String>> {
    if paths.is_empty() {
        bail!("Select at least one file; an empty selection never means all files");
    }
    let mut targets = BTreeSet::new();
    for input in paths {
        let p = c.relative(input)?;
        let full = c.target(&p)?;
        if full.starts_with(&c.source)
            || full.starts_with(&c.state)
            || Path::new(&p).components().any(|x| x.as_os_str() == ".git")
        {
            bail!("Cannot manage dot state or Git internals: {p}");
        }
        if full.is_dir() && !fs::symlink_metadata(&full)?.file_type().is_symlink() {
            if capture {
                targets.extend(ignores::eligible(c, std::slice::from_ref(&p))?);
                targets.extend(
                    s.managed
                        .keys()
                        .filter(|k| k.starts_with(&format!("{p}/")))
                        .cloned(),
                );
            } else {
                targets.extend(
                    s.managed
                        .keys()
                        .filter(|k| k.starts_with(&format!("{p}/")))
                        .cloned(),
                );
            }
        } else if capture || s.managed.contains_key(&p) {
            targets.insert(p);
        } else {
            // missing directory containing managed entries
            targets.extend(
                s.managed
                    .keys()
                    .filter(|k| k.starts_with(&format!("{p}/")))
                    .cloned(),
            );
        }
    }
    if capture {
        let candidates: Vec<_> = targets
            .iter()
            .filter(|p| !s.managed.contains_key(*p))
            .cloned()
            .collect();
        let eligible = ignores::eligible(c, &candidates)?;
        targets.retain(|p| s.managed.contains_key(p) || eligible.contains(p));
    }
    targets.retain(|p| {
        let full = c.home.join(p);
        !full.starts_with(&c.source)
            && !full.starts_with(&c.state)
            && !Path::new(p)
                .components()
                .any(|part| part.as_os_str() == ".git")
    });
    if targets.is_empty() {
        return Err(NoEligibleFiles.into());
    }
    if targets.len() > 10000 {
        bail!("Select a smaller folder (more than 10000 eligible files)");
    }
    for p in &targets {
        c.target(p)?;
    }
    Ok(targets.into_iter().collect())
}
pub fn preview(c: &Config, paths: &[String], reverse: bool) -> Result<String> {
    if paths.is_empty() {
        return Ok("Select a file to inspect its diff.".into());
    }
    let mut args = process::strings(&["diff", "--recursive=false"]);
    if reverse {
        args.push("--reverse".into());
    }
    args.push("--".into());
    for p in paths {
        args.push(c.target(p)?.to_string_lossy().into_owned());
    }
    chez_args(c, args)
}
