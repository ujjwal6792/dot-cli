//! Values (spec §7): variables with a default and per-target overrides, and
//! the references that tracked files use to name them.
//!
//! A variable lives in its own data file (`home/.chezmoidata/dotwarden-values/
//! <NAME>.toml`), so two computers changing different variables never conflict
//! textually. A file with references is stored as a template in which every
//! `{{` of ordinary content is escaped, so literal braces survive. Staging
//! maps the machine's resolved values back to references (`reverse`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::assign::{self, Known};
use crate::changes::{Change, State};
use crate::lock::Lock;
use crate::pending::{self, Action, FileChange, Item as Pending, Preview};
use crate::repo;
use crate::source::Tracked;
use crate::store::Store;
use crate::track::{self, Ctx, HomePath, new_item, refuse};
use crate::variants::{self, Pick, Variant, Variants};

/// Variable files, relative to the repository root.
pub const DIR: &str = "home/.chezmoidata/dotwarden-values";
/// Every reference (and so every template written for references) contains this.
pub const MARKER: &str = "{{ includeTemplate \"dotwarden/value\"";
const REF_HEAD: &str = "{{ includeTemplate \"dotwarden/value\" (dict \"data\" . \"name\" \"";
const REF_TAIL: &str = "\") }}";
const ESC2: &str = "{{ \"{{\" }}";
const ESC1: &str = "{{ \"{\" }}";
/// Detection ignores values shorter than this.
const DETECT_MIN: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub target: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Var {
    pub name: String,
    pub default: String,
    pub overrides: Vec<Override>,
}

impl Var {
    /// The value a machine with `targets` receives and where it comes from
    /// (`None` is the default).
    pub fn pick(&self, targets: &[String]) -> Result<(String, Option<String>)> {
        let entries: Vec<Variant> = self
            .overrides
            .iter()
            .map(|o| Variant {
                target: o.target.clone(),
                file: o.value.clone(),
            })
            .collect();
        match variants::pick(&entries, targets) {
            Pick::Default => Ok((self.default.clone(), None)),
            Pick::One(v) => Ok((v.file.clone(), Some(v.target.clone()))),
            Pick::Clash(t) => Err(refuse(format!(
                "overrides of {} clash at the same specificity: {}",
                self.name,
                t.join(", ")
            ))),
        }
    }
}

pub fn file_of(name: &str) -> String {
    format!("{DIR}/{name}.toml")
}

pub fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(crate::error::DwError::usage(format!(
            "invalid variable name {name:?}: use 1-64 letters, digits, `_` or `-`"
        ))
        .into())
    }
}

pub fn check_value(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(crate::error::DwError::usage(
            "a value is 1-1024 characters on one line without control characters",
        )
        .into());
    }
    Ok(())
}

/// Every variable in the repository.
pub fn read_all(repo: &Path) -> Result<BTreeMap<String, Var>> {
    let mut out = BTreeMap::new();
    let Ok(dir) = fs::read_dir(repo.join(DIR)) else {
        return Ok(out);
    };
    let mut files: Vec<_> = dir.flatten().map(|e| e.path()).collect();
    files.sort();
    for p in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
    {
        let text = fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let data: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        let table = data
            .get("dotwarden")
            .and_then(|d| d.get("values"))
            .and_then(toml::Value::as_table);
        for (name, v) in table.into_iter().flatten() {
            if v.get("kind")
                .and_then(toml::Value::as_str)
                .unwrap_or("variable")
                != "variable"
            {
                continue;
            }
            let Some(default) = v.get("default").and_then(toml::Value::as_str) else {
                continue;
            };
            let mut overrides: Vec<Override> = v
                .get("overrides")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    Some(Override {
                        target: o.get("target")?.as_str()?.to_string(),
                        value: o.get("value")?.as_str()?.to_string(),
                    })
                })
                .collect();
            overrides.sort_by(|a, b| a.target.cmp(&b.target));
            out.insert(
                name.clone(),
                Var {
                    name: name.clone(),
                    default: default.to_string(),
                    overrides,
                },
            );
        }
    }
    Ok(out)
}

fn write_var(repo: &Path, v: &Var) -> Result<()> {
    let mut entry = toml::Table::new();
    entry.insert("kind".into(), "variable".into());
    entry.insert("default".into(), v.default.clone().into());
    if !v.overrides.is_empty() {
        let list = v
            .overrides
            .iter()
            .map(|o| {
                let mut t = toml::Table::new();
                t.insert("target".into(), o.target.clone().into());
                t.insert("value".into(), o.value.clone().into());
                toml::Value::Table(t)
            })
            .collect();
        entry.insert("overrides".into(), toml::Value::Array(list));
    }
    let mut values = toml::Table::new();
    values.insert(v.name.clone(), toml::Value::Table(entry));
    let mut dw = toml::Table::new();
    dw.insert("values".into(), toml::Value::Table(values));
    let mut root = toml::Table::new();
    root.insert("dotwarden".into(), toml::Value::Table(dw));
    let path = repo.join(file_of(&v.name));
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    crate::fsutil::write_atomic(&path, toml::to_string(&root)?)
}

fn remove_var(repo: &Path, name: &str) -> Result<()> {
    let p = repo.join(file_of(name));
    if p.is_file() {
        fs::remove_file(&p)?;
    }
    let _ = fs::remove_dir(repo.join(DIR));
    Ok(())
}

/// Resolved values for this machine.
pub struct Resolver {
    pub vars: BTreeMap<String, Var>,
    /// Secrets (names only; values come from the vault).
    pub secrets: BTreeMap<String, crate::secrets::Meta>,
    pub targets: Vec<String>,
    /// The unlocked vault identity, when there is a session.
    identity: Option<age::x25519::Identity>,
    store: Store,
}

impl Resolver {
    pub fn load(store: &Store) -> Result<Self> {
        let vars = read_all(&store.repo())?;
        let secrets = crate::secrets::read_meta(&store.repo());
        let identity = if secrets.is_empty() {
            None
        } else {
            crate::vault::session::identity(store)
        };
        let targets = variants::this_targets(store)?.unwrap_or_default();
        Ok(Self {
            vars,
            secrets,
            targets,
            identity,
            store: store.clone(),
        })
    }

    pub fn is_secret(&self, name: &str) -> bool {
        self.secrets.contains_key(name)
    }

    pub fn unlocked(&self) -> bool {
        self.identity.is_some()
    }

    /// The value this machine receives. A secret needs the unlock session.
    pub fn value(&self, name: &str) -> Result<String> {
        if self.is_secret(name) {
            let id = self.identity.as_ref().ok_or_else(crate::vault::locked)?;
            return crate::secrets::value_for(&self.store, id, name, &self.targets);
        }
        let v = self
            .vars
            .get(name)
            .ok_or_else(|| refuse(format!("variable {name} does not exist")))?;
        Ok(v.pick(&self.targets)?.0)
    }

    /// What may be shown of a value: secrets only as their marker.
    pub fn shown(&self, name: &str, value: &str) -> String {
        if self.is_secret(name) {
            crate::secrets::marker(name)
        } else {
            value.to_string()
        }
    }

    /// The override target this machine's value comes from (`None`: default).
    pub fn source_of(&self, name: &str) -> Option<String> {
        if let Some(m) = self.secrets.get(name) {
            let shadow = Var {
                name: name.into(),
                default: String::new(),
                overrides: m
                    .targets
                    .iter()
                    .map(|t| Override {
                        target: t.clone(),
                        value: String::new(),
                    })
                    .collect(),
            };
            return shadow.pick(&self.targets).ok().and_then(|p| p.1);
        }
        self.vars
            .get(name)
            .and_then(|v| v.pick(&self.targets).ok())
            .and_then(|(_, t)| t)
    }
}

// ---- references in text ------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Lit(String),
    Ref(String),
}

pub fn reference(name: &str) -> String {
    format!("{REF_HEAD}{name}{REF_TAIL}")
}

fn push_lit(segs: &mut Vec<Seg>, s: &str) {
    if s.is_empty() {
        return;
    }
    match segs.last_mut() {
        Some(Seg::Lit(l)) => l.push_str(s),
        _ => segs.push(Seg::Lit(s.to_string())),
    }
}

/// Split a template written by dotwarden into literal text and references;
/// `None` when it holds any other template action.
pub fn parse(text: &str) -> Option<Vec<Seg>> {
    let mut segs = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        push_lit(&mut segs, &rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix(ESC2) {
            push_lit(&mut segs, "{{");
            rest = r;
        } else if let Some(r) = rest.strip_prefix(ESC1) {
            push_lit(&mut segs, "{");
            rest = r;
        } else if let Some(r) = rest.strip_prefix(REF_HEAD) {
            let end = r.find(REF_TAIL)?;
            let name = &r[..end];
            check_name(name).ok()?;
            segs.push(Seg::Ref(name.to_string()));
            rest = &r[end + REF_TAIL.len()..];
        } else {
            return None;
        }
    }
    push_lit(&mut segs, rest);
    Some(segs)
}

/// Escape literal text for a template: every `{{`, and a final `{` that is
/// followed by an action (it would otherwise start one).
fn escape(lit: &str, before_action: bool) -> String {
    let mut out = String::new();
    let mut rest = lit;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("{{") {
            out.push_str(ESC2);
            rest = &rest[2..];
        } else if c == '{' && before_action && rest.len() == 1 {
            out.push_str(ESC1);
            rest = &rest[1..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

pub fn to_template(segs: &[Seg]) -> String {
    let mut out = String::new();
    for (i, s) in segs.iter().enumerate() {
        match s {
            Seg::Lit(l) => out.push_str(&escape(l, i + 1 < segs.len())),
            Seg::Ref(n) => out.push_str(&reference(n)),
        }
    }
    out
}

/// Replace every occurrence of `value` in literal text by a reference to `name`.
pub(crate) fn wire_segs(segs: &[Seg], name: &str, value: &str) -> (Vec<Seg>, usize) {
    let mut out = Vec::new();
    let mut count = 0;
    for s in segs {
        match s {
            Seg::Lit(l) => {
                for (i, part) in l.split(value).enumerate() {
                    if i > 0 {
                        out.push(Seg::Ref(name.to_string()));
                        count += 1;
                    }
                    push_lit(&mut out, part);
                }
            }
            r => out.push(r.clone()),
        }
    }
    (out, count)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VarChange {
    pub name: String,
    pub old: String,
    pub new: String,
}

pub struct Mapped {
    pub segs: Vec<Seg>,
    pub changes: Vec<VarChange>,
}

/// Map HOME content back onto the stored template `old`: values this machine
/// resolved become references again; where the text at a reference position
/// differs, the reference is kept and the variable is reported as changed.
pub fn reverse(
    old: &[Seg],
    resolve: &dyn Fn(&str) -> Result<String>,
    is_secret: &dyn Fn(&str) -> bool,
    home: &str,
) -> Result<Mapped, String> {
    let mut rendered = String::new();
    // (name, start, end, value) in characters of `rendered`.
    let mut spans: Vec<(String, usize, usize, String)> = Vec::new();
    let mut at = 0;
    for s in old {
        match s {
            Seg::Lit(l) => {
                rendered.push_str(l);
                at += l.chars().count();
            }
            Seg::Ref(n) => {
                let v = resolve(n).map_err(|e| format!("{e:#}"))?;
                let len = v.chars().count();
                spans.push((n.clone(), at, at + len, v.clone()));
                rendered.push_str(&v);
                at += len;
            }
        }
    }
    let home_c: Vec<char> = home.chars().collect();
    let n = at;
    // Where each rendered character starts and ends in HOME.
    let mut start = vec![0usize; n];
    let mut end = vec![0usize; n];
    let mut touched = vec![false; n];
    // Insertions at a boundary between rendered characters: (new start, new end).
    let mut inserted: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_secs(2))
        .diff_chars(rendered.as_str(), home);
    for op in diff.ops() {
        match *op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    start[old_index + k] = new_index + k;
                    end[old_index + k] = new_index + k + 1;
                }
            }
            similar::DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => {
                for k in 0..old_len {
                    start[old_index + k] = new_index;
                    end[old_index + k] = new_index;
                    touched[old_index + k] = true;
                }
            }
            similar::DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => {
                inserted.insert(old_index, (new_index, new_index + new_len));
            }
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for k in 0..old_len {
                    start[old_index + k] = new_index;
                    end[old_index + k] = new_index + new_len;
                    touched[old_index + k] = true;
                }
            }
        }
    }
    let mut out: Vec<Seg> = Vec::new();
    let mut cursor = 0;
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let mut changes: BTreeMap<String, VarChange> = BTreeMap::new();
    for (name, s, e, value) in &spans {
        let (mut ns, mut ne) = (start[*s], end[*e - 1]);
        if touched[*s..*e].iter().any(|t| *t) {
            // An edited value: the whole edited region is the new value,
            // however many characters happen to coincide with the old one.
            ns = (*s..*e).map(|c| start[c]).min().unwrap_or(ns);
            ne = (*s..*e).map(|c| end[c]).max().unwrap_or(ne);
            if let Some((a, _)) = inserted.get(s) {
                ns = ns.min(*a);
            }
            if let Some((_, b)) = inserted.get(e) {
                ne = ne.max(*b);
            }
        }
        if ns < cursor || ne < ns {
            return Err(format!(
                "cannot tell where the edit around variable {name} starts and ends; \
                 change the variable with `dw values set-variable` or edit one place at a time"
            ));
        }
        push_lit(&mut out, &home_c[cursor..ns].iter().collect::<String>());
        let new: String = home_c[ns..ne].iter().collect();
        cursor = ne;
        if new.is_empty() {
            continue; // the reference was deleted
        }
        if new.chars().count() > 1024 || new.chars().any(char::is_control) {
            return Err(format!(
                "the edit at variable {name} is too large to be a value; \
                 change the variable with `dw values set-variable`"
            ));
        }
        if let Some(prev) = seen.get(name)
            && *prev != new
        {
            return Err(if is_secret(name) {
                format!("secret {name} has different text at different positions")
            } else {
                format!(
                    "variable {name} has different text at different positions ({prev:?} and {new:?})"
                )
            });
        }
        seen.insert(name.clone(), new.clone());
        if new != *value {
            changes.insert(
                name.clone(),
                VarChange {
                    name: name.clone(),
                    old: value.clone(),
                    new,
                },
            );
        }
        out.push(Seg::Ref(name.clone()));
    }
    push_lit(&mut out, &home_c[cursor..].iter().collect::<String>());
    Ok(Mapped {
        segs: out,
        changes: changes.into_values().collect(),
    })
}

// ---- where references live ---------------------------------------------

/// Names referenced by each tracked file: HOME path to names.
fn references(
    repo: &Path,
    tracked: &Tracked,
    vars: &Variants,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut add = |path: &str, text: &str| {
        if let Some(segs) = parse(text) {
            for s in segs {
                if let Seg::Ref(n) = s {
                    out.entry(path.to_string()).or_default().insert(n);
                }
            }
        }
    };
    for (path, e) in &tracked.files {
        if vars.0.contains_key(path) {
            for f in vars.files_of(repo, path) {
                if let Ok(t) = fs::read_to_string(repo.join(f))
                    && t.contains(MARKER)
                {
                    add(path, &t);
                }
            }
        } else if e.source.ends_with(".tmpl")
            && let Ok(t) = fs::read_to_string(repo.join(&e.source))
        {
            add(path, &t);
        }
    }
    out
}

/// Variable name to the HOME paths whose stored files reference it.
pub fn usage(repo: &Path, tracked: &Tracked) -> Result<BTreeMap<String, Vec<String>>> {
    let vars = Variants::read(repo)?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, names) in references(repo, tracked, &vars) {
        for n in names {
            out.entry(n).or_default().push(path.clone());
        }
    }
    Ok(out)
}

/// The stored file that holds the reference template of `rel` on this machine:
/// the variant file received (a variant), or the `.tmpl` source. `None` when
/// the file is plain.
pub fn template_file(
    ctx: &Ctx,
    vars: &Variants,
    mine: &[String],
    rel: &str,
) -> Option<(String, bool)> {
    if vars.0.contains_key(rel) {
        let file = match variants::pick(vars.get(rel), mine) {
            Pick::One(v) => v.file.clone(),
            Pick::Default => variants::default_file(rel),
            Pick::Clash(_) => return None,
        };
        return Some((variants::repo_path(&file), true));
    }
    let e = ctx.tracked.files.get(rel)?;
    e.source
        .ends_with(".tmpl")
        .then(|| (e.source.clone(), false))
}

// ---- staging -----------------------------------------------------------

/// What staging a changed file with references writes.
pub struct RefStage {
    pub rel: String,
    /// The repository file written.
    pub dest: String,
    pub bytes: Vec<u8>,
    pub changes: Vec<VarChange>,
    /// Nothing but the file mode differs.
    pub unchanged: bool,
    /// Vault files written with it (a plain file that gained secrets).
    pub extra: Vec<(String, Vec<u8>)>,
    /// The plain source it replaces.
    pub remove: Option<String>,
}

/// Plan staging `rel` when its stored file `stored` (a variant file or a
/// `.tmpl` source) holds references; `Ok(None)` when it does not.
pub fn stage_plan(
    store: &Store,
    resolver: &Resolver,
    rel: &str,
    stored: &str,
    variant: bool,
) -> Result<Option<RefStage>, String> {
    let text = fs::read_to_string(store.repo().join(stored)).map_err(|e| e.to_string())?;
    if (variant && !text.contains(MARKER)) || (!variant && !stored.ends_with(".tmpl")) {
        return Ok(None);
    }
    if let Some(id) = crate::render::file_id(&text) {
        return crate::encfile::stage_plan(store, rel, id).map(Some);
    }
    let segs = parse(&text)
        .ok_or("is a template that dotwarden did not write; staging it is not supported")?;
    let home = fs::read_to_string(store.dirs.home.join(rel))
        .map_err(|_| "is not UTF-8 text; files with references hold text".to_string())?;
    let mapped = reverse(
        &segs,
        &|n| resolver.value(n),
        &|n| resolver.is_secret(n),
        &home,
    )?;
    let refs = mapped.segs.iter().any(|s| matches!(s, Seg::Ref(_)));
    let bytes = if refs || !variant {
        to_template(&mapped.segs)
    } else if home.contains(MARKER) {
        return Err("contains text that looks like a reference; it cannot be stored raw".into());
    } else {
        home
    };
    Ok(Some(RefStage {
        rel: rel.to_string(),
        dest: stored.to_string(),
        unchanged: bytes == text && mapped.changes.is_empty(),
        bytes: bytes.into_bytes(),
        changes: mapped.changes,
        extra: Vec::new(),
        remove: None,
    }))
}

/// What staging a changed value updates: the default or the override of the
/// most specific target this machine matched.
fn update_target(resolver: &Resolver, name: &str) -> String {
    resolver
        .source_of(name)
        .map_or("the default".to_string(), |t| format!("the {t} override"))
}

/// "variable X changed: a → b (staging updates …)".
pub fn describe(resolver: &Resolver, c: &VarChange) -> String {
    if resolver.is_secret(&c.name) {
        return format!(
            "secret {} changed (staging updates {} in the vault)",
            c.name,
            update_target(resolver, &c.name)
        );
    }
    format!(
        "variable {} changed: {} → {} (staging updates {})",
        c.name,
        c.old,
        c.new,
        update_target(resolver, &c.name)
    )
}

/// Write the new values of staged changes at the most specific target this
/// machine matched.
pub fn apply_changes(store: &Store, changes: &[VarChange]) -> Result<()> {
    let repo = store.repo();
    let resolver = Resolver::load(store)?;
    let mut by_name: BTreeMap<&str, &VarChange> = BTreeMap::new();
    for c in changes {
        by_name.insert(&c.name, c);
    }
    for (name, c) in by_name {
        if resolver.is_secret(name) {
            crate::secrets::update_from_stage(store, name, &c.new, &resolver.targets)?;
            continue;
        }
        let mut v = resolver
            .vars
            .get(name)
            .cloned()
            .with_context(|| format!("variable {name} does not exist"))?;
        match v.pick(&resolver.targets)?.1 {
            None => v.default = c.new.clone(),
            Some(t) => {
                if let Some(o) = v.overrides.iter_mut().find(|o| o.target == t) {
                    o.value = c.new.clone();
                }
            }
        }
        write_var(&repo, &v)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueChange {
    pub path: String,
    pub name: String,
    pub from: String,
    pub to: String,
    pub updates: String,
    /// A secret: `from` and `to` are markers, never values.
    pub secret: bool,
}

/// The variables whose value HOME holds differently at a reference position,
/// for local changes.
pub fn value_changes(store: &Store, ctx: &Ctx, changes: &[Change]) -> Result<Vec<ValueChange>> {
    let resolver = Resolver::load(store)?;
    if resolver.vars.is_empty() && resolver.secrets.is_empty() {
        return Ok(Vec::new());
    }
    let vars = Variants::read(&store.repo())?;
    let mut out = Vec::new();
    for c in changes
        .iter()
        .filter(|c| c.state == State::Local && c.kind == "modified")
    {
        let Some((stored, variant)) = template_file(ctx, &vars, &resolver.targets, &c.path) else {
            continue;
        };
        if let Ok(Some(plan)) = stage_plan(store, &resolver, &c.path, &stored, variant) {
            for ch in &plan.changes {
                out.push(ValueChange {
                    path: c.path.clone(),
                    name: ch.name.clone(),
                    from: resolver.shown(&ch.name, &ch.old),
                    to: resolver.shown(&ch.name, &ch.new),
                    updates: update_target(&resolver, &ch.name),
                    secret: resolver.is_secret(&ch.name),
                });
            }
        }
    }
    Ok(out)
}

// ---- planning ----------------------------------------------------------

fn tilde(rel: &str) -> String {
    format!("~/{rel}")
}

fn change(action: &str, target: &str, source: &str) -> FileChange {
    FileChange {
        action: action.into(),
        target: target.into(),
        source: source.into(),
        kind: "file".into(),
    }
}

fn key_of(name: &str, target: Option<&str>) -> String {
    match target {
        Some(t) => format!("{name}@{t}"),
        None => name.to_string(),
    }
}

/// What registry machines receive before and after a change of `name`.
fn effects(
    registry: &[crate::machine::Machine],
    name: &str,
    before: Option<&Var>,
    after: Option<&Var>,
) -> Vec<String> {
    let value = |v: Option<&Var>, t: &[String]| {
        v.map_or("(none)".to_string(), |v| {
            v.pick(t).map_or_else(|e| format!("{e}"), |p| p.0)
        })
    };
    registry
        .iter()
        .filter(|m| !m.retired)
        .filter_map(|m| {
            let t = assign::machine_targets(m);
            let (b, a) = (value(before, &t), value(after, &t));
            (a != b).then(|| format!("{}: {name} {b} → {a}", m.name))
        })
        .collect()
}

fn used_note(store: &Store, ctx: &Ctx, name: &str) -> Result<Option<String>> {
    let used = usage(&store.repo(), &ctx.tracked)?;
    Ok(used.get(name).map(|files| {
        format!(
            "used by {}",
            files
                .iter()
                .map(|f| tilde(f))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }))
}

/// Plan setting the default (no target) or one override of a variable.
pub fn plan_set(
    store: &Store,
    ctx: &Ctx,
    name: &str,
    target: Option<&str>,
    value: &str,
    requester: &str,
) -> Result<Pending> {
    check_name(name)?;
    check_value(value)?;
    let repo = store.repo();
    if let Some(k) = crate::secrets::read_meta(&repo)
        .keys()
        .find(|k| k.eq_ignore_ascii_case(name))
    {
        return Err(refuse(format!(
            "{k} is a secret; its value is set with `dw values add-secret {k}` in a terminal"
        )));
    }
    let registry = repo::read_registry(&repo)?;
    let vars = read_all(&repo)?;
    let mut notes = Vec::new();
    if let Some(t) = target
        && let Some(w) = Known::from_registry(&registry).check(t)?
    {
        notes.push(format!("warning: {w}"));
    }
    let before = vars.get(name);
    let old = match (before, target) {
        (None, Some(_)) => {
            return Err(refuse(format!(
                "variable {name} has no default yet; set the default first (without --target)"
            )));
        }
        (None, None) => {
            if let Some(k) = vars.keys().find(|k| k.eq_ignore_ascii_case(name)) {
                return Err(refuse(format!(
                    "variable {k} exists; names that differ only by case would share a file"
                )));
            }
            None
        }
        (Some(v), None) => Some(v.default.clone()),
        (Some(v), Some(t)) => v
            .overrides
            .iter()
            .find(|o| o.target == t)
            .map(|o| o.value.clone()),
    };
    if old.as_deref() == Some(value) {
        return Err(refuse(format!(
            "{name} already has this value{}",
            target.map_or(String::new(), |t| format!(" for {t}"))
        )));
    }
    let mut after = before.cloned().unwrap_or(Var {
        name: name.into(),
        default: value.into(),
        overrides: Vec::new(),
    });
    match target {
        None => after.default = value.into(),
        Some(t) => match after.overrides.iter_mut().find(|o| o.target == t) {
            Some(o) => o.value = value.into(),
            None => {
                after.overrides.push(Override {
                    target: t.into(),
                    value: value.into(),
                });
                after.overrides.sort_by(|a, b| a.target.cmp(&b.target));
            }
        },
    }
    let scope = target.map_or("default".to_string(), |t| format!("override for {t}"));
    let data = vec![format!(
        "{name}: {scope} {} → {value}",
        old.as_deref().unwrap_or("(none)")
    )];
    notes.extend(used_note(store, ctx, name)?);
    let preview = Preview {
        files: vec![change(
            if before.is_some() { "change" } else { "add" },
            name,
            &file_of(name),
        )],
        data,
        notes,
        effects: effects(&registry, name, before, Some(&after)),
        ..Preview::default()
    };
    let summary = match target {
        Some(t) => format!("Set variable {name} to {value} for {t}"),
        None => format!("Set variable {name} to {value}"),
    };
    let action = Action::ValueSet {
        key: key_of(name, target),
        name: name.into(),
        target: target.map(str::to_string),
        value: value.into(),
    };
    new_item(store, requester, action, summary, preview, ctx)
}

/// Plan removing one override, or (no target) the whole variable.
pub fn plan_unset(
    store: &Store,
    ctx: &Ctx,
    name: &str,
    target: Option<&str>,
    requester: &str,
) -> Result<Pending> {
    let repo = store.repo();
    let registry = repo::read_registry(&repo)?;
    let vars = read_all(&repo)?;
    let before = vars.get(name).ok_or_else(|| {
        if crate::secrets::read_meta(&repo).contains_key(name) {
            refuse(format!("{name} is a secret, not a variable"))
        } else {
            refuse(format!("variable {name} does not exist"))
        }
    })?;
    let mut after = before.clone();
    let (data, files, mut notes);
    match target {
        Some(t) => {
            if !after.overrides.iter().any(|o| o.target == t) {
                return Err(refuse(format!("{name} has no override for {t}")));
            }
            after.overrides.retain(|o| o.target != t);
            data = vec![format!("{name}: remove the override for {t}")];
            files = vec![change("change", name, &file_of(name))];
            notes = Vec::new();
        }
        None => {
            if let Some(n) = used_note(store, ctx, name)? {
                return Err(refuse(format!(
                    "{name} is still referenced ({n}); the references would no longer render"
                )));
            }
            data = vec![format!("remove variable {name} and its overrides")];
            files = vec![change("remove", name, &file_of(name))];
            notes = Vec::new();
        }
    }
    if target.is_some() {
        notes.extend(used_note(store, ctx, name)?);
    }
    let preview = Preview {
        files,
        data,
        notes,
        effects: effects(&registry, name, Some(before), target.map(|_| &after)),
        ..Preview::default()
    };
    let summary = match target {
        Some(t) => format!("Remove the override of {name} for {t}"),
        None => format!("Remove variable {name}"),
    };
    let action = Action::ValueUnset {
        key: key_of(name, target),
        name: name.into(),
        target: target.map(str::to_string),
    };
    new_item(store, requester, action, summary, preview, ctx)
}

/// The repository changes of wiring one variable into one file.
struct Wiring {
    writes: Vec<(String, String)>,
    removes: Vec<String>,
    count: usize,
    lines: Vec<usize>,
    value: String,
    /// What may be shown of `value` (a marker for secrets).
    shown: String,
    variant: bool,
}

/// `known` is a value just found in a file (a new secret): used instead of
/// resolving `name`.
fn wiring(store: &Store, ctx: &Ctx, path: &str, name: &str, known: Option<&str>) -> Result<Wiring> {
    let repo = store.repo();
    let resolver = Resolver::load(store)?;
    if !resolver.vars.contains_key(name) && !resolver.is_secret(name) {
        return Err(refuse(format!("variable {name} does not exist")));
    }
    if resolver.targets.is_empty() {
        return Err(refuse("this machine is not in the registry".into()));
    }
    let value = match known {
        Some(v) => v.to_string(),
        None => resolver.value(name)?,
    };
    let shown = resolver.shown(name, &value);
    let entry = ctx.tracked.files.get(path).ok_or_else(|| {
        refuse(format!(
            "{} is not a tracked file; track it first",
            tilde(path)
        ))
    })?;
    if entry.symlink {
        return Err(refuse(format!("{} is a symlink", tilde(path))));
    }
    if entry
        .source
        .rsplit('/')
        .next()
        .is_some_and(|n| n.contains("encrypted_"))
    {
        return Err(refuse(format!("{} is encrypted", tilde(path))));
    }
    let vars = Variants::read(&repo)?;
    let read = |rel: &str| -> Result<String> {
        fs::read_to_string(repo.join(rel))
            .map_err(|_| refuse(format!("{} is not readable UTF-8 text", tilde(path))))
    };
    let (stored, variant) = if vars.0.contains_key(path) {
        let file = match variants::pick(vars.get(path), &resolver.targets) {
            Pick::One(v) => v.file.clone(),
            Pick::Default => variants::default_file(path),
            Pick::Clash(t) => {
                return Err(refuse(format!(
                    "variants of {} clash: {}",
                    tilde(path),
                    t.join(", ")
                )));
            }
        };
        (variants::repo_path(&file), true)
    } else {
        (entry.source.clone(), false)
    };
    let text = read(&stored)?;
    let is_template = if variant {
        text.contains(MARKER)
    } else {
        stored.ends_with(".tmpl")
    };
    let segs = if is_template {
        parse(&text).ok_or_else(|| {
            refuse(format!(
                "{} is a template that dotwarden did not write",
                tilde(path)
            ))
        })?
    } else {
        vec![Seg::Lit(text)]
    };
    let (wired, count) = wire_segs(&segs, name, &value);
    if count == 0 {
        return Err(refuse(format!(
            "the value of {name} on this machine ({}) does not occur in the stored {} \
             (a value that only exists in HOME needs `dw stage` first)",
            if resolver.is_secret(name) {
                "hidden".to_string()
            } else {
                format!("{shown:?}")
            },
            tilde(path)
        )));
    }
    let rendered: String = segs
        .iter()
        .map(|s| match s {
            Seg::Lit(l) => l.clone(),
            Seg::Ref(n) => resolver.value(n).unwrap_or_default(),
        })
        .collect();
    let lines = rendered
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains(&value))
        .map(|(i, _)| i + 1)
        .collect();
    let body = to_template(&wired);
    let (mut writes, mut removes) = (Vec::new(), Vec::new());
    if variant {
        writes.push((stored, body));
        let tmpl = variants::template(path);
        if fs::read_to_string(repo.join(&entry.source)).ok().as_deref() != Some(tmpl.as_str()) {
            writes.push((entry.source.clone(), tmpl));
        }
    } else if stored.ends_with(".tmpl") {
        writes.push((stored, body));
    } else {
        writes.push((format!("{stored}.tmpl"), body));
        removes.push(stored);
    }
    Ok(Wiring {
        writes,
        removes,
        count,
        lines,
        value,
        shown,
        variant,
    })
}

/// Plan replacing this machine's value of `name` in a tracked file by references.
pub fn plan_wire(
    store: &Store,
    ctx: &Ctx,
    hp: &HomePath,
    name: &str,
    requester: &str,
) -> Result<Pending> {
    let w = wiring(store, ctx, &hp.rel, name, None)?;
    let mut files = Vec::new();
    for r in &w.removes {
        files.push(change("remove", &hp.rel, r));
    }
    for (p, _) in &w.writes {
        let exists = store.repo().join(p).is_file();
        files.push(change(if exists { "change" } else { "add" }, &hp.rel, p));
    }
    let shown: Vec<String> = w.lines.iter().take(10).map(usize::to_string).collect();
    let mut notes = vec![
        format!(
            "{} occurrence(s) of {:?} on line(s) {} become {{{{ includeTemplate \"dotwarden/value\" … \"{name}\" }}}}",
            w.count,
            w.shown,
            shown.join(", ")
        ),
        format!(
            "{} renders the same text on this machine; HOME is not changed",
            tilde(&hp.rel)
        ),
    ];
    if w.variant {
        notes.push("only the variant this machine receives gains references".into());
    } else {
        notes.push("the stored file becomes a template; literal `{{` is escaped".into());
    }
    if w.value.chars().count() < DETECT_MIN {
        notes.push(format!(
            "warning: {:?} is short; check that every occurrence is meant",
            w.shown
        ));
    }
    let preview = Preview {
        files,
        notes,
        ..Preview::default()
    };
    let action = Action::ValueWire {
        key: format!("{}@{name}", hp.rel),
        path: hp.rel.clone(),
        name: name.into(),
    };
    new_item(
        store,
        requester,
        action,
        format!("Wire {name} into {}", tilde(&hp.rel)),
        preview,
        ctx,
    )
}

/// What a value item's outcome depends on.
pub fn guard(store: &Store, ctx: &Ctx, action: &Action) -> Result<BTreeMap<String, String>> {
    let repo = store.repo();
    let mut g = BTreeMap::new();
    let (name, path) = match action {
        Action::ValueSet { name, .. } | Action::ValueUnset { name, .. } => (name, None),
        Action::ValueWire { name, path, .. } => (name, Some(path)),
        _ => return Ok(g),
    };
    g.insert(
        format!("value:{name}"),
        track::repo_fingerprint(&repo, &file_of(name)),
    );
    if let Some(p) = path {
        let vars = Variants::read(&repo)?;
        if let Some(e) = ctx.tracked.files.get(p) {
            g.insert(
                format!("tracked:{p}"),
                track::repo_fingerprint(&repo, &e.source),
            );
        }
        for f in vars.files_of(&repo, p) {
            g.insert(format!("variant:{f}"), track::repo_fingerprint(&repo, &f));
        }
    }
    Ok(g)
}

fn write_repo(repo: &Path, rel: &str, text: &str) -> Result<()> {
    let p = repo.join(rel);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(&p, text).with_context(|| format!("writing {}", p.display()))
}

/// Make the repository change of a value item; returns the files that change.
pub fn exec(store: &Store, action: &Action) -> Result<BTreeSet<String>> {
    let repo = store.repo();
    match action {
        Action::ValueSet {
            name,
            target,
            value,
            ..
        } => {
            let mut v = read_all(&repo)?.remove(name).unwrap_or(Var {
                name: name.clone(),
                default: value.clone(),
                overrides: Vec::new(),
            });
            match target {
                None => v.default = value.clone(),
                Some(t) => match v.overrides.iter_mut().find(|o| &o.target == t) {
                    Some(o) => o.value = value.clone(),
                    None => {
                        v.overrides.push(Override {
                            target: t.clone(),
                            value: value.clone(),
                        });
                        v.overrides.sort_by(|a, b| a.target.cmp(&b.target));
                    }
                },
            }
            write_var(&repo, &v)?;
            Ok(BTreeSet::from([file_of(name)]))
        }
        Action::ValueUnset { name, target, .. } => {
            match target {
                None => remove_var(&repo, name)?,
                Some(t) => {
                    let mut v = read_all(&repo)?
                        .remove(name)
                        .with_context(|| format!("variable {name} no longer exists"))?;
                    v.overrides.retain(|o| &o.target != t);
                    write_var(&repo, &v)?;
                }
            }
            Ok(BTreeSet::from([file_of(name)]))
        }
        Action::ValueWire { path, name, .. } => wire_in_repo(store, path, name, None),
        _ => unreachable!("not a value action"),
    }
}

/// Write the wiring of `name` into the file `path`; returns the files changed.
pub fn wire_in_repo(
    store: &Store,
    path: &str,
    name: &str,
    known: Option<&str>,
) -> Result<BTreeSet<String>> {
    let repo = store.repo();
    let ctx = Ctx::load(store)?;
    let w = wiring(store, &ctx, path, name, known)?;
    let mut files = BTreeSet::new();
    for (rel, text) in &w.writes {
        write_repo(&repo, rel, text)?;
        files.insert(rel.clone());
    }
    for rel in &w.removes {
        fs::remove_file(repo.join(rel))?;
        files.insert(rel.clone());
    }
    Ok(files)
}

/// A renamed machine keeps its overrides; returns the files rewritten.
pub fn rename_machine(repo: &Path, old: &str, new: &str) -> Result<BTreeSet<String>> {
    let (from, to) = (format!("machine:{old}"), format!("machine:{new}"));
    let mut files = BTreeSet::new();
    for (name, mut v) in read_all(repo)? {
        if v.overrides.iter().any(|o| o.target == from) {
            for o in v.overrides.iter_mut().filter(|o| o.target == from) {
                o.target = to.clone();
            }
            v.overrides.sort_by(|a, b| a.target.cmp(&b.target));
            write_var(repo, &v)?;
            files.insert(file_of(&name));
        }
    }
    Ok(files)
}

// ---- detection ---------------------------------------------------------

/// After tracking or staging `rels`: propose (never apply) wiring every
/// variable whose value this machine holds in their content. Returns notes.
pub fn propose_wires(store: &Store, lock: &Lock, rels: &[String]) -> Result<Vec<String>> {
    let resolver = Resolver::load(store)?;
    let ctx = Ctx::load(store)?;
    let requester = pending::requester(store);
    let mut items = Vec::new();
    let names: Vec<&String> = if resolver.targets.is_empty() {
        Vec::new()
    } else {
        resolver
            .vars
            .keys()
            .chain(resolver.secrets.keys())
            .collect()
    };
    for rel in rels {
        let Ok(text) = fs::read_to_string(store.dirs.home.join(rel)) else {
            continue;
        };
        for name in &names {
            // Secrets can only be compared while unlocked.
            let Ok(value) = resolver.value(name) else {
                continue;
            };
            if value.chars().count() < DETECT_MIN || !text.contains(&value) {
                continue;
            }
            let hp = HomePath {
                abs: store.dirs.home.join(rel),
                rel: rel.clone(),
            };
            if let Ok(item) = plan_wire(store, &ctx, &hp, name, &requester) {
                items.push(item);
            }
        }
        items.extend(crate::detect::moves(store, &ctx, rel, &text, &requester));
    }
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let check = track::stale_checker(store, &ctx);
    Ok(pending::add(store, lock, items, &check)?
        .into_iter()
        .filter(|a| !a.already_pending)
        .map(|a| format!("proposed {}: {}", a.item.label(), a.item.summary))
        .collect())
}

// ---- listing -----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub name: String,
    pub kind: &'static str,
    pub default: String,
    pub overrides: Vec<Override>,
    /// The value this machine receives and where it comes from.
    pub here: Option<String>,
    pub here_from: Option<String>,
    pub files: Vec<String>,
}

pub fn list(store: &Store) -> Result<Vec<Row>> {
    let repo = store.repo();
    let resolver = Resolver::load(store)?;
    let used = usage(&repo, &source_tracked(&repo)?)?;
    let mut rows: Vec<Row> = resolver
        .vars
        .values()
        .map(|v| {
            let here = (!resolver.targets.is_empty())
                .then(|| v.pick(&resolver.targets).ok())
                .flatten();
            Row {
                name: v.name.clone(),
                kind: "variable",
                default: v.default.clone(),
                overrides: v.overrides.clone(),
                here_from: here
                    .as_ref()
                    .map(|(_, t)| t.clone().unwrap_or("default".into())),
                here: here.map(|(v, _)| v),
                files: used.get(&v.name).cloned().unwrap_or_default(),
            }
        })
        .collect();
    for m in resolver.secrets.values() {
        let has_value = !m.placeholder && !resolver.targets.is_empty();
        rows.push(Row {
            name: m.name.clone(),
            kind: "secret",
            default: if m.placeholder {
                "(placeholder: no value yet)".into()
            } else {
                crate::secrets::MASK.into()
            },
            overrides: m
                .targets
                .iter()
                .map(|t| Override {
                    target: t.clone(),
                    value: crate::secrets::MASK.into(),
                })
                .collect(),
            here: has_value.then(|| crate::secrets::MASK.to_string()),
            here_from: has_value.then(|| {
                resolver
                    .source_of(&m.name)
                    .unwrap_or_else(|| "default".into())
            }),
            files: used.get(&m.name).cloned().unwrap_or_default(),
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Texts to diff for a changed file that needs the vault: what it renders with
/// markers instead of secrets, and HOME with secrets masked. `None` for files
/// that do not need the vault.
pub fn masked_diff(
    store: &Store,
    ctx: &Ctx,
    resolver: &Resolver,
    vars: &Variants,
    rel: &str,
    home: &str,
) -> Option<(String, String)> {
    let (stored, variant) = template_file(ctx, vars, &resolver.targets, rel)?;
    let text = fs::read_to_string(store.repo().join(&stored)).ok()?;
    if variant && !text.contains(MARKER) {
        return None;
    }
    if crate::render::file_id(&text).is_some() {
        return Some((
            "«encrypted file»\n".into(),
            "«encrypted file (content hidden)»\n".into(),
        ));
    }
    let segs = parse(&text)?;
    if !segs
        .iter()
        .any(|s| matches!(s, Seg::Ref(n) if resolver.is_secret(n)))
    {
        return None;
    }
    let mut old = String::new();
    let mut known = Vec::new();
    for s in &segs {
        match s {
            Seg::Lit(l) => old.push_str(l),
            Seg::Ref(n) if resolver.is_secret(n) => {
                old.push_str(&crate::secrets::marker(n));
                if let Ok(v) = resolver.value(n) {
                    known.push((n.clone(), v));
                }
            }
            Seg::Ref(n) => old.push_str(&resolver.value(n).unwrap_or_default()),
        }
    }
    let masked = mask_home(&old, home, &known);
    Some((old, masked))
}

const HIDDEN: &str = "«secret position: HOME text hidden»\n";

/// Does `line` match `pattern` (a rendered line holding markers), where each
/// marker stands for any text?
fn line_matches(pattern: &str, line: &str) -> bool {
    let mut parts = Vec::new();
    let mut rest = pattern;
    while let Some(i) = rest.find("«secret:") {
        parts.push(&rest[..i]);
        match rest[i..].find('»') {
            Some(j) => rest = &rest[i + j + '»'.len_utf8()..],
            None => break,
        }
    }
    parts.push(rest);
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if line.len() < first.len() + last.len() || !line.starts_with(first) || !line.ends_with(last) {
        return false;
    }
    let mut at = first.len();
    let end = line.len() - last.len();
    for p in &parts[1..parts.len() - 1] {
        match line[at..end].find(p) {
            Some(i) => at += i + p.len(),
            None => return false,
        }
    }
    true
}

/// HOME text with every secret masked: known values become markers, and where
/// the rendered text has a marker line HOME's line (whatever it holds there)
/// is shown as that marker line, or hidden when the edit is not a plain
/// replacement.
pub fn mask_home(rendered: &str, home: &str, known: &[(String, String)]) -> String {
    let mut text = home.to_string();
    let mut by_len: Vec<_> = known.iter().filter(|(_, v)| !v.is_empty()).collect();
    by_len.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (n, v) in by_len {
        text = text.replace(v.as_str(), &crate::secrets::marker(n));
    }
    let old_lines: Vec<&str> = rendered.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = text.split_inclusive('\n').collect();
    let diff = similar::TextDiff::from_slices(&old_lines, &new_lines);
    let mut out = String::new();
    for op in diff.ops() {
        match op.tag() {
            similar::DiffTag::Equal => {
                for l in &new_lines[op.new_range()] {
                    out.push_str(l);
                }
            }
            _ => {
                let (o, n) = (op.old_range(), op.new_range());
                let secret_here = old_lines[o.clone()].iter().any(|l| l.contains("«secret:"));
                if !secret_here {
                    for l in &new_lines[n] {
                        out.push_str(l);
                    }
                } else if o.len() == n.len() {
                    for (a, b) in o.zip(n) {
                        out.push_str(if !old_lines[a].contains("«secret:") {
                            new_lines[b]
                        } else if line_matches(old_lines[a], new_lines[b]) {
                            old_lines[a]
                        } else {
                            HIDDEN
                        });
                    }
                } else if !n.is_empty() {
                    out.push_str(HIDDEN);
                }
            }
        }
    }
    out
}

/// A secret's decrypted default and overrides: for the TUI Values screen
/// only. Deliberately not `Serialize`.
#[derive(Debug, Clone)]
pub struct Reveal {
    pub default: String,
    pub overrides: Vec<Override>,
}

/// Decrypted secrets while the vault is unlocked (empty when locked).
pub fn reveal(store: &Store) -> BTreeMap<String, Reveal> {
    let Some(id) = crate::vault::session::identity(store) else {
        return BTreeMap::new();
    };
    crate::secrets::read_meta(&store.repo())
        .values()
        .filter(|m| !m.placeholder)
        .filter_map(|m| {
            let d = crate::secrets::load_doc(store, &id, &m.name).ok()?;
            Some((
                m.name.clone(),
                Reveal {
                    default: d.default,
                    overrides: d.overrides,
                },
            ))
        })
        .collect()
}

/// Files that cannot be rendered right now because the vault is locked.
pub fn locked_files(store: &Store) -> Result<BTreeSet<String>> {
    if !crate::vault::exists(store) || crate::vault::is_unlocked(store) {
        return Ok(BTreeSet::new());
    }
    let repo = store.repo();
    vault_files(&repo, &crate::source::load(&repo)?)
}

/// HOME paths whose stored files need the vault to render: references to
/// secrets and encrypted files.
pub fn vault_files(repo: &Path, tracked: &Tracked) -> Result<BTreeSet<String>> {
    let secrets = crate::secrets::read_meta(repo);
    let vars = Variants::read(repo)?;
    let mut out = BTreeSet::new();
    for (path, names) in references(repo, tracked, &vars) {
        if names.iter().any(|n| secrets.contains_key(n)) {
            out.insert(path);
        }
    }
    for (path, e) in &tracked.files {
        if e.source.ends_with(".tmpl")
            && fs::read_to_string(repo.join(&e.source))
                .is_ok_and(|t| t.starts_with(crate::render::FILE_MARKER))
        {
            out.insert(path.clone());
        }
    }
    Ok(out)
}

fn source_tracked(repo: &Path) -> Result<Tracked> {
    crate::source::load(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(text: &str) {
        let segs = vec![Seg::Lit(text.to_string())];
        let t = to_template(&segs);
        assert_eq!(parse(&t), Some(segs), "{text:?} via {t:?}");
    }

    #[test]
    fn braces_survive_escaping() {
        for t in [
            "plain",
            "{{ x }}",
            "{{{",
            "a{",
            "{{",
            "}}{{}}",
            "{{{{ \"{{\" }}",
        ] {
            roundtrip(t);
        }
        let segs = vec![
            Seg::Lit("a{".into()),
            Seg::Ref("X".into()),
            Seg::Lit("{{b".into()),
        ];
        assert_eq!(parse(&to_template(&segs)), Some(segs));
        assert_eq!(parse("{{ .foo }}"), None, "foreign actions are refused");
    }

    #[test]
    fn reverse_keeps_references_and_reports_changed_values() {
        let old = vec![
            Seg::Lit("size = ".into()),
            Seg::Ref("SIZE".into()),
            Seg::Lit("\nfont = ".into()),
            Seg::Ref("FONT".into()),
            Seg::Lit("\n".into()),
        ];
        let resolve =
            |n: &str| -> Result<String> { Ok(if n == "SIZE" { "12" } else { "Mono" }.to_string()) };
        let same = reverse(&old, &resolve, &|_| false, "size = 12\nfont = Mono\n").unwrap();
        assert_eq!(same.segs, old);
        assert!(same.changes.is_empty());
        let edited = reverse(&old, &resolve, &|_| false, "size = 14\nfont = Mono # x\n").unwrap();
        assert_eq!(edited.changes.len(), 1);
        assert_eq!(
            (
                edited.changes[0].old.as_str(),
                edited.changes[0].new.as_str()
            ),
            ("12", "14")
        );
        assert_eq!(edited.segs.last(), Some(&Seg::Lit(" # x\n".into())));
    }
}
