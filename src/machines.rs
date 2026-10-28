//! Machines and assignments as pending items (spec §4): assigning targets to
//! tracked paths, adding, renaming and retiring registry machines, and
//! previewing what another machine would receive.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use crate::assign::{self, Assignment, Assignments, Known, Side};
use crate::error::DwError;
use crate::machine::{self, Machine};
use crate::pending::{Action, AssignChange, Item as Pending, Preview};
use crate::repo::{self, DATA_FILE, MACHINES_DIR};
use crate::store::Store;
use crate::track::{Ctx, HomePath, new_item, refuse};

fn registry_file(name: &str) -> String {
    format!("{MACHINES_DIR}/{name}.toml")
}

fn tilde(rel: &str) -> String {
    format!("~/{rel}")
}

#[derive(Debug, Clone, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub machine: Machine,
    pub this_machine: bool,
}

/// The registry with this computer marked.
pub fn list(store: &Store) -> Result<Vec<Listed>> {
    let me = store.local_machine_name()?;
    Ok(repo::read_registry(&store.repo())?
        .into_iter()
        .map(|m| Listed {
            this_machine: me.as_deref() == Some(m.name.as_str()),
            machine: m,
        })
        .collect())
}

// ---- assign --------------------------------------------------------------

/// One edit of the assignment editor or `dw assign`: put the target on a side,
/// or (`None`) on neither.
pub type Edit = (String, Option<Side>);

fn describe_edits(edits: &[Edit], clear: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if clear {
        parts.push("clear".into());
    }
    for (t, side) in edits {
        parts.push(match side {
            Some(Side::Include) => format!("include {t}"),
            Some(Side::Exclude) => format!("exclude {t}"),
            None => format!("unset {t}"),
        });
    }
    parts.join(", ")
}

/// Plan assigning: apply `edits` (after `clear`) to the assignment of each
/// path. A path must be tracked, or a folder holding tracked files.
pub fn plan_assign(
    store: &Store,
    ctx: &Ctx,
    paths: &[HomePath],
    edits: &[Edit],
    clear: bool,
    requester: &str,
) -> Result<Pending> {
    let repo = store.repo();
    let registry = repo::read_registry(&repo)?;
    let known = Known::from_registry(&registry);
    let mut notes = Vec::new();
    for (t, side) in edits {
        if side.is_some() {
            if let Some(w) = known.check(t)? {
                notes.push(format!("warning: {w}"));
            }
        } else {
            assign::validate_target(t)?;
        }
    }
    for (i, (t, _)) in edits.iter().enumerate() {
        if edits[..i].iter().any(|(o, _)| o == t) {
            return Err(DwError::usage(format!("{t} is named twice")).into());
        }
    }
    let before = assign::read_assignments(&repo)?;
    let mut after = before.clone();
    let mut changes: Vec<AssignChange> = Vec::new();
    let mut errors = Vec::new();
    for hp in paths {
        let rel = &hp.rel;
        let tracked = ctx.tracked.is_tracked(rel)
            || ctx.tracked.folders.contains(rel)
            || ctx.tracked.under(rel).next().is_some();
        if !tracked {
            errors.push(format!(
                "{} is not tracked (track it first, or name a tracked file or folder)",
                tilde(rel)
            ));
            continue;
        }
        if changes.iter().any(|c| &c.path == rel) {
            continue;
        }
        assign::validate_target_path(rel)?;
        let cur = before.get(rel).cloned().unwrap_or_default();
        let mut new = if clear {
            Assignment::default()
        } else {
            cur.clone()
        };
        for (t, side) in edits {
            assign::set_side(&mut new, t, *side);
        }
        if new == cur {
            continue;
        }
        if new.is_empty() {
            after.remove(rel);
        } else {
            after.insert(rel.clone(), new.clone());
        }
        changes.push(AssignChange {
            path: rel.clone(),
            include: new.include,
            exclude: new.exclude,
        });
    }
    if !errors.is_empty() {
        return Err(refuse(errors.join("\n")));
    }
    if changes.is_empty() {
        return Err(refuse(
            "nothing to change: the paths are already assigned that way".to_string(),
        ));
    }
    let changed: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
    let data: Vec<String> = changes
        .iter()
        .map(|c| {
            let was = before.get(&c.path).cloned().unwrap_or_default();
            let now = Assignment {
                include: c.include.clone(),
                exclude: c.exclude.clone(),
            };
            format!(
                "{}: {} → {}",
                tilde(&c.path),
                was.describe(),
                now.describe()
            )
        })
        .collect();
    for c in &changes {
        let mut up = c.path.as_str();
        while let Some((parent, _)) = up.rsplit_once('/') {
            if before.contains_key(parent) {
                notes.push(format!(
                    "{} is also governed by the assignment of the folder {}",
                    tilde(&c.path),
                    tilde(parent)
                ));
                break;
            }
            up = parent;
        }
        if ctx.tracked.folders.contains(&c.path) || ctx.tracked.under(&c.path).count() > 1 {
            notes.push(format!(
                "{} is a folder: the assignment applies to everything below it",
                tilde(&c.path)
            ));
        }
    }
    let preview = Preview {
        data,
        notes,
        effects: assign::effects(&ctx.tracked, &registry, &before, &after, &changed),
        ..Preview::default()
    };
    let shown = if changed.len() == 1 {
        tilde(&changed[0])
    } else {
        format!("{} paths", changed.len())
    };
    let summary = format!("Assign {shown} ({})", describe_edits(edits, clear));
    let key = changes
        .iter()
        .map(|c| {
            let a = Assignment {
                include: c.include.clone(),
                exclude: c.exclude.clone(),
            };
            format!("{}={}", c.path, a.fingerprint())
        })
        .collect::<Vec<_>>()
        .join("\n");
    new_item(
        store,
        requester,
        Action::Assign { key, changes },
        summary,
        preview,
        ctx,
    )
}

// ---- add, rename, retire -------------------------------------------------

fn check_word(what: &str, v: &str) -> Result<()> {
    assign::validate_target(&format!("distribution:{v}")).map_err(|_| {
        DwError::usage(format!(
            "invalid {what} {v:?}: use a-z, 0-9, '-', '.' and '_'"
        ))
    })?;
    Ok(())
}

pub fn plan_add(store: &Store, ctx: &Ctx, machine: Machine, requester: &str) -> Result<Pending> {
    machine::validate_name(&machine.name)?;
    if !["darwin", "linux"].contains(&machine.platform.as_str()) {
        return Err(DwError::usage(format!(
            "unknown platform {:?}: use darwin or linux",
            machine.platform
        ))
        .into());
    }
    check_word("distribution", &machine.distribution)?;
    for f in &machine.families {
        check_word("family", f)?;
    }
    let registry = repo::read_registry(&store.repo())?;
    if registry.iter().any(|m| m.name == machine.name) {
        return Err(refuse(format!(
            "machine {:?} is already in the registry",
            machine.name
        )));
    }
    let preview = Preview {
        data: vec![format!(
            "register {} ({}/{}, families: {})",
            machine.name,
            machine.platform,
            machine.distribution,
            if machine.families.is_empty() {
                "none".to_string()
            } else {
                machine.families.join(", ")
            }
        )],
        notes: vec![format!("writes {}", registry_file(&machine.name))],
        ..Preview::default()
    };
    let summary = format!("Register machine {}", machine.name);
    new_item(
        store,
        requester,
        Action::MachineAdd { machine },
        summary,
        preview,
        ctx,
    )
}

/// Replace the string `machine:<old>` (and keys of that name) everywhere in
/// the data file; returns where it was found.
fn rename_refs(doc: &mut DocumentMut, old: &str, new: &str) -> Vec<String> {
    let (from, to) = (format!("machine:{old}"), format!("machine:{new}"));
    let mut out = Vec::new();
    let mut at = Vec::new();
    walk_table(doc.as_table_mut(), &from, &to, &mut at, &mut out);
    out
}

fn walk_table(t: &mut Table, from: &str, to: &str, at: &mut Vec<String>, out: &mut Vec<String>) {
    if from != to
        && let Some(item) = t.remove(from)
    {
        t.insert(to, item);
        out.push(format!("{} / {from} (key)", at.join(" / ")));
    }
    let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
    for k in keys {
        at.push(k.clone());
        if let Some(item) = t.get_mut(&k) {
            match item {
                Item::Table(sub) => walk_table(sub, from, to, at, out),
                Item::ArrayOfTables(a) => {
                    for sub in a.iter_mut() {
                        walk_table(sub, from, to, at, out);
                    }
                }
                Item::Value(v) => walk_value(v, from, to, at, out),
                Item::None => {}
            }
        }
        at.pop();
    }
}

fn walk_inline(
    t: &mut InlineTable,
    from: &str,
    to: &str,
    at: &mut Vec<String>,
    out: &mut Vec<String>,
) {
    if from != to
        && let Some(v) = t.remove(from)
    {
        t.insert(to, v);
        out.push(format!("{} / {from} (key)", at.join(" / ")));
    }
    let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
    for k in keys {
        at.push(k.clone());
        if let Some(v) = t.get_mut(&k) {
            walk_value(v, from, to, at, out);
        }
        at.pop();
    }
}

fn walk_value(v: &mut Value, from: &str, to: &str, at: &mut Vec<String>, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.value() == from => {
            out.push(at.join(" / "));
            if from != to {
                *v = Value::from(to);
            }
        }
        Value::Array(a) => {
            for x in a.iter_mut() {
                walk_value(x, from, to, at, out);
            }
        }
        Value::InlineTable(t) => walk_inline(t, from, to, at, out),
        _ => {}
    }
}

fn data_doc(store: &Store) -> Result<DocumentMut> {
    let path = store.repo().join(DATA_FILE);
    fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))
}

pub fn plan_rename(
    store: &Store,
    ctx: &Ctx,
    old: &str,
    new: &str,
    requester: &str,
) -> Result<Pending> {
    machine::validate_name(new)?;
    let registry = repo::read_registry(&store.repo())?;
    let Some(m) = registry.iter().find(|m| m.name == old) else {
        return Err(refuse(format!("machine {old:?} is not in the registry")));
    };
    if m.retired {
        return Err(refuse(format!("machine {old:?} is retired")));
    }
    if registry.iter().any(|m| m.name == new) {
        return Err(refuse(format!(
            "the name {new:?} is already in the registry"
        )));
    }
    let mut doc = data_doc(store)?;
    let refs = rename_refs(&mut doc, old, new);
    let mut data = vec![format!(
        "registry: {} → {}",
        registry_file(old),
        registry_file(new)
    )];
    data.extend(
        refs.iter()
            .map(|r| format!("machine:{old} → machine:{new} in {r}")),
    );
    let note = if store.local_machine_name()?.as_deref() == Some(old) {
        "this machine: machine.toml and the generated chezmoi config are updated".to_string()
    } else {
        format!(
            "the computer registered as {old} no longer finds itself in the registry after it pulls; it must be registered again under {new}"
        )
    };
    let preview = Preview {
        data,
        notes: vec![note],
        ..Preview::default()
    };
    let summary = format!(
        "Rename machine {old} to {new} ({} reference(s))",
        refs.len()
    );
    new_item(
        store,
        requester,
        Action::MachineRename {
            old: old.into(),
            new: new.into(),
        },
        summary,
        preview,
        ctx,
    )
}

/// The assignments with `machine:<name>` taken out of every list, except that
/// a file whose only include it is keeps it: a retired machine never matches,
/// so the file then applies to no machine instead of to all of them.
fn without_machine(before: &Assignments, name: &str) -> Assignments {
    let target = format!("machine:{name}");
    let mut after = Assignments::new();
    for (p, a) in before {
        let mut a = a.clone();
        if a.include != [target.clone()] {
            a.include.retain(|t| *t != target);
        }
        a.exclude.retain(|t| *t != target);
        if !a.is_empty() {
            after.insert(p.clone(), a);
        }
    }
    after
}

pub fn plan_retire(store: &Store, ctx: &Ctx, name: &str, requester: &str) -> Result<Pending> {
    let registry = repo::read_registry(&store.repo())?;
    let Some(m) = registry.iter().find(|m| m.name == name) else {
        return Err(refuse(format!("machine {name:?} is not in the registry")));
    };
    if m.retired {
        return Err(refuse(format!("machine {name:?} is already retired")));
    }
    if store.local_machine_name()?.as_deref() == Some(name) {
        return Err(refuse(format!(
            "{name:?} is this machine; retire it from another machine"
        )));
    }
    let before = assign::read_assignments(&store.repo())?;
    let after = without_machine(&before, name);
    let target = format!("machine:{name}");
    let mut data = Vec::new();
    let mut touched = Vec::new();
    for (p, a) in &before {
        let sides: Vec<&str> = [("include", &a.include), ("exclude", &a.exclude)]
            .iter()
            .filter(|(_, l)| l.contains(&target))
            .map(|(s, _)| *s)
            .collect();
        if sides.is_empty() {
            continue;
        }
        let now = after.get(p).cloned().unwrap_or_default();
        let nowhere = a.include == [target.clone()];
        let result = if nowhere {
            format!(
                "kept as the only include; {} applies to no machine now",
                tilde(p)
            )
        } else {
            format!(
                "remove {target} from {} → {}",
                sides.join(" and "),
                now.describe()
            )
        };
        data.push(format!(
            "{}: {result} ({} tracked file(s))",
            tilde(p),
            ctx.tracked.under(p).count()
        ));
        touched.push(p.clone());
    }
    data.push(format!("registry: {name} is marked retired and kept"));
    let mut doc = data_doc(store)?;
    let notes = rename_refs(&mut doc, name, name)
        .into_iter()
        .filter(|r| !r.starts_with("assignments / "))
        .map(|r| {
            format!(
                "{r} still names {target}; it never matches a retired machine, remove it by hand"
            )
        })
        .collect();
    let active: Vec<Machine> = registry
        .iter()
        .filter(|m| m.name != name)
        .cloned()
        .collect();
    let preview = Preview {
        data,
        notes,
        effects: assign::effects(&ctx.tracked, &active, &before, &after, &touched),
        ..Preview::default()
    };
    let summary = format!(
        "Retire machine {name} ({} assignment(s) change)",
        touched.len()
    );
    new_item(
        store,
        requester,
        Action::MachineRetire { name: name.into() },
        summary,
        preview,
        ctx,
    )
}

// ---- guards and execution -------------------------------------------------

fn registry_state(registry: &[Machine], name: &str) -> String {
    registry
        .iter()
        .find(|m| m.name == name)
        .map_or("-".to_string(), |m| {
            format!(
                "{}/{}/{}/{}",
                m.platform,
                m.distribution,
                m.families.join(","),
                m.retired
            )
        })
}

fn data_hash(store: &Store) -> String {
    let text = fs::read(store.repo().join(DATA_FILE)).unwrap_or_default();
    Sha256::digest(&text)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The guard entries (repository side) of the machine and assignment actions.
pub fn guard(store: &Store, action: &Action) -> Result<BTreeMap<String, String>> {
    let repo = store.repo();
    let registry = repo::read_registry(&repo)?;
    let mut g = BTreeMap::new();
    let reg = |name: &str, g: &mut BTreeMap<String, String>| {
        g.insert(format!("registry:{name}"), registry_state(&registry, name));
    };
    match action {
        Action::Assign { changes, .. } => {
            let now = assign::read_assignments(&repo)?;
            for c in changes {
                g.insert(
                    format!("assigned:{}", c.path),
                    now.get(&c.path).map_or("-".into(), Assignment::fingerprint),
                );
                for t in c.include.iter().chain(&c.exclude) {
                    if let Some(m) = t.strip_prefix("machine:") {
                        reg(m, &mut g);
                    }
                }
            }
        }
        Action::MachineAdd { machine } => reg(&machine.name, &mut g),
        Action::MachineRename { old, new } => {
            reg(old, &mut g);
            reg(new, &mut g);
            g.insert("data:file".into(), data_hash(store));
        }
        Action::MachineRetire { name } => {
            reg(name, &mut g);
            g.insert("data:file".into(), data_hash(store));
        }
        _ => {}
    }
    Ok(g)
}

/// Make the repository change of a machine or assignment action. Returns the
/// repository files that must change (the data file may also).
pub fn exec(store: &Store, action: &Action) -> Result<BTreeSet<String>> {
    let repo = store.repo();
    let mut registry = repo::read_registry(&repo)?;
    match action {
        Action::Assign { changes, .. } => {
            for c in changes {
                assign::set_assignment(&repo, &c.path, &c.include, &c.exclude)?;
            }
            Ok(BTreeSet::from([DATA_FILE.to_string()]))
        }
        Action::MachineAdd { machine } => {
            registry.push(machine.clone());
            repo::write_registry(&repo, &registry)?;
            Ok(BTreeSet::from([registry_file(&machine.name)]))
        }
        Action::MachineRename { old, new } => {
            let m = registry
                .iter_mut()
                .find(|m| m.name == *old)
                .with_context(|| format!("machine {old:?} is no longer in the registry"))?;
            m.name = new.clone();
            repo::write_registry(&repo, &registry)?;
            let (from, to) = (old.clone(), new.clone());
            repo::edit_data(&repo, |doc| {
                rename_refs(doc, &from, &to);
                Ok(())
            })?;
            Ok(BTreeSet::from([registry_file(old), registry_file(new)]))
        }
        Action::MachineRetire { name } => {
            let m = registry
                .iter_mut()
                .find(|m| m.name == *name)
                .with_context(|| format!("machine {name:?} is no longer in the registry"))?;
            m.retired = true;
            repo::write_registry(&repo, &registry)?;
            let before = assign::read_assignments(&repo)?;
            let after = without_machine(&before, name);
            for (p, b) in &before {
                if after.get(p) != Some(b) {
                    let a = after.get(p).cloned().unwrap_or_default();
                    assign::set_assignment(&repo, p, &a.include, &a.exclude)?;
                }
            }
            Ok(BTreeSet::from([registry_file(name)]))
        }
        _ => unreachable!("not a machine action"),
    }
}

/// After the change is staged: a renamed machine that is this one follows.
pub fn finish(store: &Store, action: &Action) -> Result<()> {
    if let Action::MachineRename { old, new } = action
        && store.local_machine_name()?.as_deref() == Some(old.as_str())
    {
        store.write_local_machine(new)?;
        store.write_chezmoi_config(new)?;
    }
    Ok(())
}

/// `dw machine use`: point this computer at an existing registry entry whose
/// platform and distribution match it (after the machine was renamed elsewhere).
pub fn use_machine(store: &Store, name: &str) -> Result<Machine> {
    let registry = repo::read_registry(&store.repo())?;
    let Some(m) = registry.into_iter().find(|m| m.name == name) else {
        return Err(refuse(format!(
            "machine {name:?} is not in the registry (see `dw machine list`)"
        )));
    };
    if m.retired {
        return Err(refuse(format!("machine {name:?} is retired")));
    }
    let here = machine::detect()?;
    if (here.platform.as_str(), here.distribution.as_str())
        != (m.platform.as_str(), m.distribution.as_str())
    {
        return Err(refuse(format!(
            "{name:?} is registered as {}/{} but this computer is {}/{}",
            m.platform, m.distribution, here.platform, here.distribution
        )));
    }
    let _lock = store.lock()?;
    store.write_local_machine(name)?;
    store.write_chezmoi_config(name)?;
    Ok(m)
}

// ---- preview as machine ------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct PreviewAs {
    pub machine: Machine,
    /// Tracked files (relative to HOME) the machine would receive.
    pub files: Vec<String>,
    /// chezmoi's diff of those files against this HOME, when asked for.
    pub diff: Option<String>,
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Render what registry machine `name` would receive, with its own throwaway
/// chezmoi state and the machine name overridden; nothing in HOME or in
/// Dotwarden's real state is written.
pub fn preview_as(store: &Store, name: &str, diff: bool) -> Result<PreviewAs> {
    let registry = repo::read_registry(&store.repo())?;
    let Some(m) = registry.iter().find(|m| m.name == name) else {
        return Err(refuse(format!("machine {name:?} is not in the registry")));
    };
    if m.retired {
        return Err(refuse(format!(
            "machine {name:?} is retired and receives nothing"
        )));
    }
    let base = store
        .env()
        .get(std::ffi::OsStr::new("TMPDIR"))
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = TempDir(base.join(format!("dw-preview-{}-{nanos}", std::process::id())));
    fs::create_dir_all(&dir.0)?;
    let config = dir.0.join("chezmoi.toml");
    fs::write(&config, Store::chezmoi_config_body(name)?)?;
    let run = |args: &[&str]| -> Result<String> {
        let mut cmd =
            store.chezmoi_command_with(&config, &dir.0.join("state.boltdb"), &dir.0.join("cache"));
        cmd.args(args);
        Ok(crate::exec::run_ok(cmd, None)?.stdout)
    };
    let mut files: Vec<String> = run(&[
        "managed",
        "--include=files,symlinks",
        "--path-style=relative",
    ])?
    .lines()
    .map(str::to_string)
    .collect();
    files.sort();
    let diff = if diff {
        Some(run(&[
            "diff",
            "--include=files,symlinks",
            "--use-builtin-diff",
            "--no-pager",
        ])?)
    } else {
        None
    };
    Ok(PreviewAs {
        machine: m.clone(),
        files,
        diff,
    })
}
