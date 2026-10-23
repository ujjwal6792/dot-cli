//! File applicability stored with its native rules in one .chezmoiignore block.
use crate::{
    backend,
    config::{Config, atomic_write, clean_relative},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const BEGIN: &str = "# dot: file assignments v1 begin\n";
const END: &str = "# dot: file assignments v1 end\n";
const DATA: &str = "# assignments: ";
pub const INVENTORY: &str = r#"{"dot_file_scope_inventory":true}"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    #[default]
    All,
    Darwin,
    Linux,
}
impl Os {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Darwin => "Darwin",
            Self::Linux => "Linux",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Darwin,
            Self::Darwin => Self::Linux,
            Self::Linux => Self::All,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub os: Os,
    /// Empty means all machines; values are chezmoi's short hostnames.
    pub machines: BTreeSet<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Identity {
    pub os: String,
    pub hostname: String,
}
impl Scope {
    pub fn allows(&self, identity: &Identity) -> bool {
        (self.os == Os::All || self.os.label().to_lowercase() == identity.os)
            && (self.machines.is_empty() || self.machines.contains(&identity.hostname))
    }
    pub fn machine_label(&self) -> String {
        if self.machines.is_empty() {
            "All machines".into()
        } else {
            self.machines.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    }
    pub fn parse_machines(text: &str) -> Result<BTreeSet<String>> {
        let mut machines = BTreeSet::new();
        if text.trim().is_empty() {
            return Ok(machines);
        }
        for value in text.split(',') {
            let value = value.trim();
            // chezmoi.hostname deliberately omits the domain after the first dot.
            let name = value.split('.').next().unwrap_or_default();
            if name.is_empty()
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                bail!("Use comma-separated hostnames, for example laptop, desktop");
            }
            machines.insert(name.into());
        }
        Ok(machines)
    }
    fn validate(&self) -> Result<()> {
        if self.machines.iter().any(|m| {
            m.is_empty()
                || !m
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        }) {
            bail!("Machine assignments require chezmoi short hostnames (without a domain)");
        }
        Ok(())
    }
}

pub fn identity(c: &Config) -> Result<Identity> {
    Ok(serde_json::from_str(&backend::chez(
        c,
        &[
            "execute-template",
            "{{ dict \"os\" .chezmoi.os \"hostname\" .chezmoi.hostname | toJson }}",
        ],
    )?)?)
}

struct Store {
    relative: String,
    before: Option<String>,
    rules: BTreeMap<String, Scope>,
}

fn literal(path: &str) -> Result<String> {
    clean_relative(Path::new(path))?;
    if path.chars().any(char::is_control)
        || path.contains("{{")
        || path.contains("}}")
        || path.contains('#')
        || path.trim() != path
        || Path::new(path)
            .components()
            .any(|p| p.as_os_str() == ".git")
    {
        bail!("This path needs a manually escaped chezmoi rule: {path:?}");
    }
    let mut pattern = String::new();
    for ch in path.chars() {
        if "\\*?[]{}! ".contains(ch) {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    Ok(pattern)
}

fn block(rules: &BTreeMap<String, Scope>) -> Result<String> {
    if rules.is_empty() {
        return Ok(String::new());
    }
    let mut result = format!("{BEGIN}{DATA}{}\n", serde_json::to_string(rules)?);
    result.push_str(
        "{{ if not (and (hasKey . \"dot_file_scope_inventory\") .dot_file_scope_inventory) }}\n",
    );
    for (path, scope) in rules {
        let pattern = literal(path)?;
        scope.validate()?;
        let mut exclusions = Vec::new();
        if scope.os != Os::All {
            exclusions.push(format!(
                "(ne .chezmoi.os {:?})",
                scope.os.label().to_lowercase()
            ));
        }
        if !scope.machines.is_empty() {
            exclusions.push(format!(
                "(not (has .chezmoi.hostname (list {})))",
                scope
                    .machines
                    .iter()
                    .map(|m| format!("{m:?}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !exclusions.is_empty() {
            let condition = if exclusions.len() == 1 {
                exclusions[0].clone()
            } else {
                format!("or {}", exclusions.join(" "))
            };
            result.push_str(&format!(
                "{{{{ if {condition} }}}}\n{pattern}\n{pattern}/**\n{{{{ end }}}}\n"
            ));
        }
    }
    result.push_str("{{ end }}\n");
    result.push_str(END);
    Ok(result)
}

fn split(text: &str) -> Result<(&str, &str, &str)> {
    let begin = text.match_indices(BEGIN).collect::<Vec<_>>();
    let end = text.match_indices(END).collect::<Vec<_>>();
    match (begin.as_slice(), end.as_slice()) {
        ([], []) => Ok((text, "", "")),
        ([(start, _)], [(finish, _)])
            if start < finish && (*start == 0 || text.as_bytes()[start - 1] == b'\n') =>
        {
            Ok((
                &text[..*start],
                &text[*start..finish + END.len()],
                &text[finish + END.len()..],
            ))
        }
        _ => bail!(
            "The dot assignment block is incomplete or duplicated; preserve it and resolve it before saving"
        ),
    }
}
fn safe_file(c: &Config, relative: &str) -> Result<PathBuf> {
    clean_relative(Path::new(relative))?;
    let mut path = c.source.clone();
    for part in Path::new(relative).components() {
        path.push(part);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "Assignment control file or parent is a symlink: {}",
                path.display()
            );
        }
    }
    Ok(path)
}
fn load(c: &Config) -> Result<Store> {
    let root = PathBuf::from(backend::chez(c, &["source-path"])?.trim());
    let plain = root.join(".chezmoiignore");
    let template = root.join(".chezmoiignore.tmpl");
    if plain.exists() && template.exists() {
        bail!("Consolidate .chezmoiignore and .chezmoiignore.tmpl before assigning files");
    }
    let path = if template.exists() { template } else { plain };
    let relative = clean_relative(
        path.strip_prefix(&c.source)
            .context("Ignore file is outside source")?,
    )?;
    let path = safe_file(c, &relative)?;
    let before = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let (_, managed, _) = split(before.as_deref().unwrap_or_default())?;
    let rules = if managed.is_empty() {
        BTreeMap::new()
    } else {
        let data = managed
            .lines()
            .nth(1)
            .and_then(|l| l.strip_prefix(DATA))
            .context("Assignment metadata missing")?;
        let rules = serde_json::from_str(data)?;
        if block(&rules)? != managed {
            bail!(
                "Dot assignment rules were edited manually; preserve and resolve the block before saving"
            );
        }
        rules
    };
    Ok(Store {
        relative,
        before,
        rules,
    })
}
pub fn read(c: &Config) -> Result<BTreeMap<String, Scope>> {
    Ok(load(c)?.rules)
}

pub fn applies(rules: &BTreeMap<String, Scope>, path: &str, identity: &Identity) -> bool {
    Path::new(path)
        .ancestors()
        .filter_map(|path| path.to_str())
        .filter_map(|path| rules.get(path))
        .all(|scope| scope.allows(identity))
}

#[derive(Serialize, Deserialize)]
pub struct Change {
    relative: String,
    before: Option<String>,
    content: String,
    pub preview: String,
}

pub fn prepare(c: &Config, paths: &[String], scope: &Scope) -> Result<Change> {
    if paths.is_empty() {
        bail!("Select a tracked file or folder to assign");
    }
    scope.validate()?;
    let mut store = load(c)?;
    let snap = backend::snapshot(c)?;
    let mut targets = Vec::new();
    let mut preview = "Change where these saved configurations apply.\nOriginal files stay unchanged; saved copies remain tracked by Git.\n\n".to_string();
    for input in paths {
        let path = c.relative(input)?;
        literal(&path)?;
        c.target(&path)?;
        let tracked: Vec<_> = snap
            .managed
            .keys()
            .chain(snap.scoped_out.keys())
            .filter(|p| *p == &path || p.starts_with(&format!("{path}/")))
            .cloned()
            .collect();
        if tracked.is_empty() {
            bail!("Capture {path} before assigning its OS or machines");
        }
        targets.extend(tracked);
        let old = store.rules.get(&path).cloned().unwrap_or_default();
        preview.push_str(&format!(
            "~/{}\n  OS: {} → {}\n  Machines: {} → {}\n",
            path,
            old.os.label(),
            scope.os.label(),
            old.machine_label(),
            scope.machine_label()
        ));
        if scope == &Scope::default() {
            store.rules.remove(&path);
        } else {
            store.rules.insert(path, scope.clone());
        }
    }
    let (prefix, _, suffix) = split(store.before.as_deref().unwrap_or_default())?;
    let new_block = block(&store.rules)?;
    let separator = if !prefix.is_empty() && !prefix.ends_with('\n') && !new_block.is_empty() {
        "\n"
    } else {
        ""
    };
    let content = format!("{prefix}{separator}{new_block}{suffix}");
    if content == store.before.as_deref().unwrap_or_default() {
        bail!("These assignments are already saved");
    }
    // Check the real native result in an isolated source, including custom ! exceptions.
    validate_native(
        c,
        &store.relative,
        &content,
        &store.rules,
        &targets,
        &snap.identity,
    )?;
    preview.push_str(&format!("\nUpdate {}\nFolder restrictions also apply to descendants. Parent assignments and other native ignore rules still apply.\n\n--- Before\n{}\n+++ After\n{}",
        store.relative, store.before.as_deref().unwrap_or_default(), content));
    Ok(Change {
        relative: store.relative,
        before: store.before,
        content,
        preview,
    })
}

fn validate_native(
    c: &Config,
    relative: &str,
    content: &str,
    rules: &BTreeMap<String, Scope>,
    targets: &[String],
    current: &Identity,
) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut isolated = c.clone();
    isolated.source = temp.path().join("source");
    isolated.state = temp.path().join("state");
    // Copy only the settings source, never HOME or Git internals.
    for entry in walkdir::WalkDir::new(&c.source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry?;
        let target = isolated.source.join(entry.path().strip_prefix(&c.source)?);
        if entry.file_type().is_dir() {
            fs::create_dir_all(target)?;
        } else if entry.file_type().is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    atomic_write(&isolated.source.join(relative), content.as_bytes())?;
    let mut hosts: BTreeSet<_> = rules
        .values()
        .flat_map(|s| s.machines.iter().cloned())
        .collect();
    hosts.insert(current.hostname.clone());
    let mut other = "dot-unassigned-machine".to_string();
    while hosts.contains(&other) {
        other.push('-');
    }
    hosts.insert(other);
    let systems = BTreeSet::from(["darwin", "linux", "windows", current.os.as_str()]);
    for os in systems {
        for host in &hosts {
            let identity = Identity {
                os: os.into(),
                hostname: host.clone(),
            };
            let forbidden: Vec<_> = targets
                .iter()
                .filter(|p| !applies(rules, p, &identity))
                .collect();
            if forbidden.is_empty() {
                continue;
            }
            let data = serde_json::json!({"chezmoi":{"os":os,"hostname":host}}).to_string();
            let managed: BTreeMap<String, backend::Managed> =
                serde_json::from_str(&backend::chez(
                    &isolated,
                    &[
                        "--override-data",
                        &data,
                        "managed",
                        "--format=json",
                        "--path-style=all",
                        "--include=files,symlinks",
                    ],
                )?)?;
            if let Some(path) = forbidden.iter().find(|p| managed.contains_key(p.as_str())) {
                bail!(
                    "An existing native include (!) rule overrides the assignment for {path}. Resolve that exception before saving"
                );
            }
        }
    }
    Ok(())
}

pub fn apply(c: &Config, payload: &str) -> Result<String> {
    let change: Change = serde_json::from_str(payload)?;
    let store = load(c)?;
    if store.relative != change.relative || store.before != change.before {
        bail!("Assignment rules changed after review");
    }
    let path = safe_file(c, &change.relative)?;
    atomic_write(&path, change.content.as_bytes())?;
    Ok("OS and machine assignments saved. Originals are unchanged; commit and push the rule change to share it.".into())
}
