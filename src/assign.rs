//! Assignments (spec §4): validation of what may be written into the data file,
//! and the edit itself. The fixed `.chezmoiignore` template evaluates them.

use std::path::Path;

use anyhow::Result;
use toml_edit::{Array, Item, Table, value};

use crate::error::DwError;
use crate::repo;

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
