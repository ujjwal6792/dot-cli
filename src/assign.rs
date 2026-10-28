//! Assignments (spec §4): validation of what may be written into the data file,
//! and the edit itself. The fixed `.chezmoiignore` template evaluates them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, Item, Table, value};

use crate::error::DwError;
use crate::machine::Machine;
use crate::repo;
use crate::source::Tracked;

pub const KINDS: [&str; 4] = ["platform", "distribution", "family", "machine"];

/// A target path must fit on one line of `.chezmoiignore` and name a place
/// under HOME: relative, normalised, without control characters or newlines.
pub fn validate_target_path(path: &str) -> Result<()> {
    let bad = |why: &str| -> Result<()> {
        Err(DwError::usage(format!("cannot assign {path:?}: {why}")).into())
    };
    if path.is_empty() {
        return bad("the path is empty");
    }
    if path.starts_with('/') {
        return bad("the path must be relative to HOME");
    }
    if path.ends_with('/') {
        return bad("the path must not end with '/'");
    }
    if path.chars().any(char::is_control) {
        return bad(
            "it contains a newline or another control character, which chezmoi's ignore file cannot represent",
        );
    }
    if path
        .split('/')
        .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return bad("the path must not contain empty, '.' or '..' components");
    }
    Ok(())
}

/// `platform:<p>`, `distribution:<d>`, `family:<f>` or `machine:<name>`.
pub fn validate_target(target: &str) -> Result<()> {
    let ok = target.split_once(':').is_some_and(|(kind, v)| {
        KINDS.contains(&kind)
            && !v.is_empty()
            && v.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || b == b'_'
            })
    });
    if !ok {
        return Err(DwError::usage(format!(
            "invalid target {target:?}: use platform:<p>, distribution:<d>, family:<f> or machine:<name>"
        ))
        .into());
    }
    Ok(())
}

fn string_array(items: &[String]) -> Array {
    items.iter().map(String::as_str).collect()
}

/// Set (or, with no includes and no excludes, remove) the assignment of a
/// target path. Only the data file changes; `.chezmoiignore` never does.
pub fn set_assignment(
    repo_dir: &Path,
    path: &str,
    include: &[String],
    exclude: &[String],
) -> Result<()> {
    validate_target_path(path)?;
    for t in include.iter().chain(exclude) {
        validate_target(t)?;
    }
    repo::edit_data(repo_dir, |doc| {
        let dw = repo::dotwarden_table(doc)?;
        let assignments = dw
            .entry("assignments")
            .or_insert_with(|| {
                let mut t = Table::new();
                t.set_implicit(true);
                Item::Table(t)
            })
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("`dotwarden.assignments` is not a table"))?;
        if include.is_empty() && exclude.is_empty() {
            assignments.remove(path);
            return Ok(());
        }
        let mut t = Table::new();
        if !include.is_empty() {
            t["include"] = value(string_array(include));
        }
        if !exclude.is_empty() {
            t["exclude"] = value(string_array(exclude));
        }
        assignments.insert(path, Item::Table(t));
        Ok(())
    })
}

/// The includes and excludes of one target path (spec §4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Assignment {
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Human text: `all machines`, or the includes and excludes.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "all machines".into();
        }
        let mut parts = Vec::new();
        if !self.include.is_empty() {
            parts.push(format!("include {}", self.include.join(", ")));
        }
        if !self.exclude.is_empty() {
            parts.push(format!("exclude {}", self.exclude.join(", ")));
        }
        parts.join("; ")
    }

    /// Stable text used to fingerprint the assignment in guards and keys.
    pub fn fingerprint(&self) -> String {
        format!("+{}|-{}", self.include.join(","), self.exclude.join(","))
    }
}

/// All assignments, keyed by target path.
pub type Assignments = BTreeMap<String, Assignment>;

pub fn read_assignments(repo_dir: &Path) -> Result<Assignments> {
    let text = match std::fs::read_to_string(repo_dir.join(repo::DATA_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Assignments::new()),
        Err(e) => return Err(e.into()),
    };
    let data: toml::Table = toml::from_str(&text)?;
    let mut out = Assignments::new();
    let Some(table) = data
        .get("dotwarden")
        .and_then(|d| d.get("assignments"))
        .and_then(toml::Value::as_table)
    else {
        return Ok(out);
    };
    let list = |v: &toml::Value, key: &str| -> Vec<String> {
        v.get(key)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect()
    };
    for (path, v) in table {
        out.insert(
            path.clone(),
            Assignment {
                include: list(v, "include"),
                exclude: list(v, "exclude"),
            },
        );
    }
    Ok(out)
}

/// The targets a registry machine matches (none when it is retired).
pub fn machine_targets(m: &Machine) -> Vec<String> {
    if m.retired {
        return Vec::new();
    }
    let mut t = vec![
        format!("platform:{}", m.platform),
        format!("distribution:{}", m.distribution),
        format!("machine:{}", m.name),
    ];
    t.extend(m.families.iter().map(|f| format!("family:{f}")));
    t
}

fn rule_applies(a: &Assignment, targets: &[String]) -> bool {
    let hit = |l: &[String]| l.iter().any(|t| targets.contains(t));
    (a.include.is_empty() || hit(&a.include)) && !hit(&a.exclude)
}

/// Whether the file at `path` applies to a machine with `targets`: the
/// assignment of the file and of every folder above it must all allow it
/// (chezmoi ignores everything below an ignored folder).
pub fn applies(assignments: &Assignments, path: &str, targets: &[String]) -> bool {
    let mut cur = path;
    loop {
        if assignments
            .get(cur)
            .is_some_and(|a| !rule_applies(a, targets))
        {
            return false;
        }
        match cur.rsplit_once('/') {
            Some((parent, _)) => cur = parent,
            None => return true,
        }
    }
}

/// Which side of an assignment a target is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Include,
    Exclude,
}

/// Put `target` on `side` (removing it from the other side), or on neither.
pub fn set_side(a: &mut Assignment, target: &str, side: Option<Side>) {
    a.include.retain(|t| t != target);
    a.exclude.retain(|t| t != target);
    match side {
        Some(Side::Include) => a.include.push(target.to_string()),
        Some(Side::Exclude) => a.exclude.push(target.to_string()),
        None => {}
    }
}

/// The values targets may take, as seen in the registry.
#[derive(Debug, Default)]
pub struct Known {
    pub distributions: BTreeSet<String>,
    pub families: BTreeSet<String>,
    pub machines: BTreeSet<String>,
    pub retired: BTreeSet<String>,
}

impl Known {
    pub fn from_registry(registry: &[Machine]) -> Self {
        let mut k = Known::default();
        for m in registry {
            if m.retired {
                k.retired.insert(m.name.clone());
            } else {
                k.machines.insert(m.name.clone());
            }
            k.distributions.insert(m.distribution.clone());
            k.families.extend(m.families.iter().cloned());
        }
        k
    }

    /// Check a target against the registry: an error for an unknown platform
    /// or machine, a warning for a distribution or family nobody has yet.
    pub fn check(&self, target: &str) -> Result<Option<String>> {
        validate_target(target)?;
        let (kind, v) = target.split_once(':').expect("validated");
        let bad = |m: String| -> Result<Option<String>> { Err(DwError::usage(m).into()) };
        match kind {
            "platform" if !["darwin", "linux"].contains(&v) => {
                bad(format!("unknown platform {v:?}: use darwin or linux"))
            }
            "machine" if self.retired.contains(v) => {
                bad(format!("machine {v:?} is retired and cannot be a target"))
            }
            "machine" if !self.machines.contains(v) => bad(format!(
                "machine {v:?} is not in the registry (see `dw machine list`)"
            )),
            "distribution" if !self.distributions.contains(v) => Ok(Some(format!(
                "no registered machine runs distribution {v:?} yet"
            ))),
            "family" if !self.families.contains(v) => Ok(Some(format!(
                "no registered machine is in family {v:?} yet"
            ))),
            _ => Ok(None),
        }
    }
}

/// State of one target across the selection in the assignment editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tri {
    None,
    Include,
    Exclude,
    /// The selected paths disagree.
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRow {
    pub group: &'static str,
    pub target: String,
    pub state: Tri,
}

fn side_of(a: Option<&Assignment>, target: &str) -> Tri {
    match a {
        Some(a) if a.include.iter().any(|t| t == target) => Tri::Include,
        Some(a) if a.exclude.iter().any(|t| t == target) => Tri::Exclude,
        _ => Tri::None,
    }
}

/// The editor's checkbox list for `paths`: every known target, grouped, with
/// its state over the selection. Targets already assigned but no longer known
/// stay listed so they can be cleared.
pub fn editor_rows(
    registry: &[Machine],
    assignments: &Assignments,
    paths: &[String],
) -> Vec<TargetRow> {
    let known = Known::from_registry(registry);
    let mut groups: [(&'static str, &str, BTreeSet<String>); 4] = [
        (
            "Platforms",
            "platform",
            ["darwin".to_string(), "linux".to_string()].into(),
        ),
        ("Distributions", "distribution", known.distributions.clone()),
        ("Families", "family", known.families.clone()),
        ("Machines", "machine", known.machines.clone()),
    ];
    for p in paths {
        for t in assignments
            .get(p)
            .into_iter()
            .flat_map(|a| a.include.iter().chain(&a.exclude))
        {
            if let Some((kind, v)) = t.split_once(':')
                && let Some(g) = groups.iter_mut().find(|g| g.1 == kind)
            {
                g.2.insert(v.to_string());
            }
        }
    }
    let mut rows = Vec::new();
    for (title, kind, values) in groups {
        for v in values {
            let target = format!("{kind}:{v}");
            let mut states = paths.iter().map(|p| side_of(assignments.get(p), &target));
            let first = states.next().unwrap_or(Tri::None);
            let state = if states.all(|s| s == first) {
                first
            } else {
                Tri::Mixed
            };
            rows.push(TargetRow {
                group: title,
                target,
                state,
            });
        }
    }
    rows
}

/// Tracked files under `paths` whose applicability changes between `before`
/// and `after` on any of `machines`, one line per file.
pub fn effects(
    tracked: &Tracked,
    machines: &[Machine],
    before: &Assignments,
    after: &Assignments,
    paths: &[String],
) -> Vec<String> {
    let files: BTreeSet<&String> = paths
        .iter()
        .flat_map(|p| tracked.under(p).map(|(t, _)| t))
        .collect();
    let mut out = Vec::new();
    for f in files {
        let (mut stops, mut starts) = (Vec::new(), Vec::new());
        for m in machines.iter().filter(|m| !m.retired) {
            let t = machine_targets(m);
            match (applies(before, f, &t), applies(after, f, &t)) {
                (true, false) => stops.push(m.name.as_str()),
                (false, true) => starts.push(m.name.as_str()),
                _ => {}
            }
        }
        let mut parts = Vec::new();
        if !stops.is_empty() {
            parts.push(format!("stops applying on {}", stops.join(", ")));
        }
        if !starts.is_empty() {
            parts.push(format!("starts applying on {}", starts.join(", ")));
        }
        if !parts.is_empty() {
            out.push(format!("~/{f}: {}", parts.join("; ")));
        }
    }
    out
}

/// Entries of assignments and variants whose target kind is unknown. Such a
/// target never matches; `dw doctor` surfaces them as warnings.
pub fn unknown_targets(repo_dir: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(repo_dir.join(repo::DATA_FILE))?;
    let data: toml::Table = toml::from_str(&text)?;
    let mut out = Vec::new();
    let known = |t: &str| t.split_once(':').is_some_and(|(k, _)| KINDS.contains(&k));
    let Some(dw) = data.get("dotwarden").and_then(toml::Value::as_table) else {
        return Ok(out);
    };
    if let Some(a) = dw.get("assignments").and_then(toml::Value::as_table) {
        for (path, entry) in a {
            for list in ["include", "exclude"] {
                for t in entry
                    .get(list)
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(t) = t.as_str().filter(|t| !known(t)) {
                        out.push(format!("assignment {path}: {t}"));
                    }
                }
            }
        }
    }
    if let Some(v) = dw.get("variants").and_then(toml::Value::as_table) {
        for (path, entries) in v {
            for e in entries.as_array().into_iter().flatten() {
                if let Some(t) = e
                    .get("target")
                    .and_then(toml::Value::as_str)
                    .filter(|t| !known(t))
                {
                    out.push(format!("variant {path}: {t}"));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_cannot_be_represented_are_refused() {
        for bad in [
            "", "/abs", "a/", "a\nb", "a\rb", "a\tb", "a/../b", "a//b", "./a", "x\u{0}y", "\u{85}",
        ] {
            assert!(validate_target_path(bad).is_err(), "{bad:?}");
        }
        for ok in [
            "#x",
            "!x",
            "a b ",
            " lead",
            "[1]",
            "*",
            "?",
            ".config/app/.nested",
            "ü\u{a0}",
        ] {
            validate_target_path(ok).unwrap();
        }
    }

    #[test]
    fn the_refusal_is_a_clear_usage_error() {
        let e = validate_target_path("a\nb").unwrap_err();
        let (kind, msg) = crate::error::classify(&e);
        assert_eq!(kind, crate::error::Kind::Usage);
        assert!(
            msg.contains("newline") && msg.contains("cannot assign"),
            "{msg}"
        );
    }

    #[test]
    fn targets() {
        for ok in [
            "platform:linux",
            "family:arch-like",
            "machine:my-box",
            "distribution:cachyos",
        ] {
            validate_target(ok).unwrap();
        }
        for bad in ["linux", "weird:x", "machine:", "machine:A", "platform:a b"] {
            assert!(validate_target(bad).is_err(), "{bad}");
        }
    }
}
