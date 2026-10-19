use crate::{
    config::{Config, clean_relative},
    git::{self, GitState},
    process,
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
    let changes = parse_status(&chez(c, &["status", "--path-style=relative"])?);
    let git = git::status(c).unwrap_or_else(|e| GitState {
        error: Some(e.to_string()),
        ..Default::default()
    });
    Ok(Snapshot {
        managed,
        changes,
        git,
        observed_at: chrono::Utc::now().to_rfc3339(),
        warnings: vec![],
    })
}
pub fn parse_status(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| l.len() >= 4 && l.is_char_boundary(3))
        .map(|l| (l[3..].to_string(), l[..2].to_string()))
        .collect()
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
                for e in walkdir::WalkDir::new(&full)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| {
                        e.file_name() != ".git"
                            && !e.path().starts_with(&c.source)
                            && !e.path().starts_with(&c.state)
                    })
                {
                    let e = e?;
                    if !e.file_type().is_dir() {
                        targets.insert(c.relative(&e.path().to_string_lossy())?);
                    }
                    if targets.len() > 10000 {
                        bail!("Select a smaller folder (more than 10000 entries)");
                    }
                }
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
    if targets.is_empty() {
        bail!("No eligible files in selection");
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
