//! Secrets (spec §7): one `vault/secrets/<NAME>.age` per secret holding the
//! default and the per-target overrides encrypted together. The name and the
//! list of override targets are metadata in the plain values data file
//! (`kind = "secret"`), so references and listings work without an unlock.
//! Nothing here ever returns a value except [`value_for`], which needs the
//! unlocked identity and is used only to render files and to compare.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use age::x25519;
use anyhow::{Context, Result};
use serde::Serialize;

use crate::store::Store;
use crate::track::refuse;
use crate::values::{self, Override, Var};
use crate::vault;

pub const MASK: &str = "••••••";

/// «secret:NAME»: how a secret appears in diffs and previews.
pub fn marker(name: &str) -> String {
    format!("«secret:{name}»")
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub name: String,
    /// Targets that have an override (their values are inside the age file).
    pub targets: Vec<String>,
    /// Created by an agent: name only, the value is set later by a human.
    pub placeholder: bool,
}

pub fn file_of(name: &str) -> String {
    vault::rel(&format!("secrets/{name}.age"))
}

pub fn read_meta(repo: &Path) -> BTreeMap<String, Meta> {
    let mut out = BTreeMap::new();
    let Ok(dir) = fs::read_dir(repo.join(values::DIR)) else {
        return out;
    };
    for p in dir.flatten().map(|e| e.path()) {
        let Some(table) = fs::read_to_string(&p)
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        else {
            continue;
        };
        let Some(vals) = table
            .get("dotwarden")
            .and_then(|d| d.get("values"))
            .and_then(toml::Value::as_table)
        else {
            continue;
        };
        for (name, v) in vals {
            if v.get("kind").and_then(toml::Value::as_str) != Some("secret") {
                continue;
            }
            let mut targets: Vec<String> = v
                .get("targets")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            targets.sort();
            out.insert(
                name.clone(),
                Meta {
                    name: name.clone(),
                    targets,
                    placeholder: v
                        .get("placeholder")
                        .and_then(toml::Value::as_bool)
                        .unwrap_or(false),
                },
            );
        }
    }
    out
}

fn meta_file(m: &Meta) -> Result<(String, Vec<u8>)> {
    let mut e = toml::Table::new();
    e.insert("kind".into(), "secret".into());
    e.insert(
        "targets".into(),
        toml::Value::Array(m.targets.iter().map(|t| t.as_str().into()).collect()),
    );
    if m.placeholder {
        e.insert("placeholder".into(), true.into());
    }
    let mut vals = toml::Table::new();
    vals.insert(m.name.clone(), e.into());
    let mut dw = toml::Table::new();
    dw.insert("values".into(), vals.into());
    let mut root = toml::Table::new();
    root.insert("dotwarden".into(), dw.into());
    Ok((
        values::file_of(&m.name),
        toml::to_string(&root)?.into_bytes(),
    ))
}

/// Decrypt one secret's default and overrides.
pub fn load_doc(store: &Store, id: &x25519::Identity, name: &str) -> Result<Var> {
    let bytes = fs::read(store.repo().join(file_of(name))).map_err(|_| {
        refuse(format!(
            "secret {name} has no value yet (a placeholder); a human sets it with `dw values add-secret {name}`"
        ))
    })?;
    let plain = vault::decrypt(id, &bytes)?;
    serde_json::from_slice(&plain).with_context(|| format!("reading secret {name}"))
}

/// The value `targets` receive: needs the unlocked identity.
pub fn value_for(
    store: &Store,
    id: &x25519::Identity,
    name: &str,
    targets: &[String],
) -> Result<String> {
    Ok(load_doc(store, id, name)?.pick(targets)?.0)
}

fn check_free(store: &Store, name: &str) -> Result<()> {
    values::check_name(name)?;
    let repo = store.repo();
    if let Some(k) = values::read_all(&repo)?
        .keys()
        .find(|k| k.eq_ignore_ascii_case(name))
    {
        return Err(refuse(format!("{k} is a variable, not a secret")));
    }
    Ok(())
}

/// Create a placeholder: the name only. Agents may do this.
pub fn add_placeholder(store: &Store, name: &str) -> Result<()> {
    check_free(store, name)?;
    vault::recipient(store)?;
    if read_meta(&store.repo()).contains_key(name) {
        return Err(refuse(format!("secret {name} already exists")));
    }
    let (p, b) = meta_file(&Meta {
        name: name.into(),
        targets: vec![],
        placeholder: true,
    })?;
    vault::write_staged(store, &[(p, b)], &[])
}

/// The repository files that setting the default or one override produces.
/// `current` is the decrypted secret, or `None` for a new one.
pub fn prepare(
    store: &Store,
    name: &str,
    target: Option<&str>,
    value: &str,
    current: Option<Var>,
) -> Result<Vec<(String, Vec<u8>)>> {
    check_free(store, name)?;
    values::check_value(value)?;
    if let Some(t) = target {
        crate::assign::validate_target(t)?;
    }
    let fresh = current.is_none();
    let mut doc = current.unwrap_or(Var {
        name: name.into(),
        default: String::new(),
        overrides: vec![],
    });
    match target {
        None => doc.default = value.into(),
        Some(_) if fresh => {
            return Err(refuse(format!(
                "secret {name} has no default yet; set the default first (without --target)"
            )));
        }
        Some(t) => match doc.overrides.iter_mut().find(|o| o.target == t) {
            Some(o) => o.value = value.into(),
            None => {
                doc.overrides.push(Override {
                    target: t.into(),
                    value: value.into(),
                });
                doc.overrides.sort_by(|a, b| a.target.cmp(&b.target));
            }
        },
    }
    let meta = Meta {
        name: name.into(),
        targets: doc.overrides.iter().map(|o| o.target.clone()).collect(),
        placeholder: false,
    };
    let (meta_path, meta_bytes) = meta_file(&meta)?;
    Ok(vec![
        (
            file_of(name),
            vault::encrypt(store, &serde_json::to_vec(&doc)?)?,
        ),
        (meta_path, meta_bytes),
    ])
}

/// Set the default or one override. A new secret needs only the recipient;
/// changing an existing one decrypts it first, so it needs an unlock.
pub fn set(store: &Store, name: &str, target: Option<&str>, value: &str) -> Result<()> {
    vault::recipient(store)?;
    let current = match read_meta(&store.repo()).get(name) {
        Some(m) if !m.placeholder => {
            let id = vault::session::require(store)?;
            Some(load_doc(store, &id, name)?)
        }
        _ => None,
    };
    let files = prepare(store, name, target, value, current)?;
    vault::write_staged(store, &files, &[])
}

/// Staging changed a secret's value at the target this machine matched.
pub fn update_from_stage(store: &Store, name: &str, new: &str, targets: &[String]) -> Result<()> {
    let id = vault::session::require(store)?;
    let doc = load_doc(store, &id, name)?;
    let target = doc.pick(targets)?.1;
    let files = prepare(store, name, target.as_deref(), new, Some(doc))?;
    let repo = store.repo();
    for (rel, bytes) in &files {
        crate::fsutil::write_atomic(&repo.join(rel), bytes)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub name: String,
    pub kind: &'static str,
    pub targets: Vec<String>,
    pub placeholder: bool,
    pub value: &'static str,
}

pub fn rows(store: &Store) -> Vec<Row> {
    read_meta(&store.repo())
        .into_values()
        .map(|m| Row {
            name: m.name,
            kind: "secret",
            targets: m.targets,
            placeholder: m.placeholder,
            value: MASK,
        })
        .collect()
}
