//! Variants (spec §3 item 6, §4): one tracked file with different stored
//! content per target. The stored file becomes a template that includes the
//! variant selected by `dotwarden/variant`; the previous content is kept as the
//! default variant. Selection is mirrored here so status, the TUI and staging
//! know what this machine receives without asking chezmoi (which fails the
//! whole run on a clash).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use toml_edit::{ArrayOfTables, Item, Table, value};

use crate::assign::{self, Known};
use crate::machine::Machine;
use crate::pending::{Action, FileChange, Item as Pending, Preview};
use crate::repo::{self, DATA_FILE};
use crate::store::Store;
use crate::track::{self, Ctx, HomePath, new_item, refuse};

/// Variant contents, relative to the repository root.
pub const DIR: &str = "home/.dotwarden/variants";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub target: String,
    /// Under `DIR`.
    pub file: String,
}

/// The variants of every tracked path, from the data file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Variants(pub BTreeMap<String, Vec<Variant>>);

pub fn repo_path(file: &str) -> String {
    format!("{DIR}/{file}")
}

pub fn default_file(path: &str) -> String {
    format!("{path}/default")
}

impl Variants {
    pub fn read(repo: &Path) -> Result<Self> {
        let text = match fs::read_to_string(repo.join(DATA_FILE)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).context("reading the data file"),
        };
        let data: toml::Table = toml::from_str(&text).context("parsing the data file")?;
        let mut out = BTreeMap::new();
        let table = data
            .get("dotwarden")
            .and_then(|d| d.get("variants"))
            .and_then(toml::Value::as_table);
        for (path, list) in table.into_iter().flatten() {
            let entries: Vec<Variant> = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    Some(Variant {
                        target: e.get("target")?.as_str()?.to_string(),
                        file: e.get("file")?.as_str()?.to_string(),
                    })
                })
                .collect();
            if !entries.is_empty() {
                out.insert(path.clone(), entries);
            }
        }
        Ok(Self(out))
    }

    pub fn get(&self, path: &str) -> &[Variant] {
        self.0.get(path).map_or(&[], Vec::as_slice)
    }

    /// Changes whenever the variants of `path` change.
    pub fn fingerprint(&self, path: &str) -> String {
        self.get(path)
            .iter()
            .map(|v| format!("{}={}", v.target, v.file))
            .collect::<Vec<_>>()
            .join(";")
    }

    /// The tracked path a repository file under `DIR` belongs to.
    pub fn target_of(&self, repo_file: &str) -> Option<String> {
        let rest = repo_file.strip_prefix(DIR)?.strip_prefix('/')?;
        for (path, entries) in &self.0 {
            if entries.iter().any(|v| v.file == rest) || default_file(path) == rest {
                return Some(path.clone());
            }
        }
        rest.rsplit_once('/').map(|(p, _)| p.to_string())
    }

    /// Repository files holding the variants (and default) of `path` that exist.
    pub fn files_of(&self, repo: &Path, path: &str) -> Vec<String> {
        self.get(path)
            .iter()
            .map(|v| v.file.clone())
            .chain([default_file(path)])
            .map(|f| repo_path(&f))
            .filter(|p| repo.join(p).is_file())
            .collect()
    }
}

fn level(target: &str) -> u8 {
    match target.split_once(':').map(|(k, _)| k) {
        Some("platform") => 1,
        Some("family") => 2,
        Some("distribution") => 3,
        Some("machine") => 4,
        _ => 0,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pick<'a> {
    Default,
    One(&'a Variant),
    /// Two matches at the best specificity: these targets.
    Clash(Vec<&'a str>),
}

/// The variant a machine with `targets` receives: machine > distribution >
/// family > platform > default; two best matches clash.
pub fn pick<'a>(entries: &'a [Variant], targets: &[String]) -> Pick<'a> {
    let hits: Vec<(u8, &Variant)> = entries
        .iter()
        .map(|v| (level(&v.target), v))
        .filter(|(l, v)| *l > 0 && targets.contains(&v.target))
        .collect();
    let Some(best) = hits.iter().map(|(l, _)| *l).max() else {
        return Pick::Default;
    };
    let top: Vec<&Variant> = hits
        .iter()
        .filter(|(l, _)| *l == best)
        .map(|(_, v)| *v)
        .collect();
    match top.as_slice() {
        [one] => Pick::One(one),
        _ => Pick::Clash(top.iter().map(|v| v.target.as_str()).collect()),
    }
}

fn describe(p: &Pick) -> String {
    match p {
        Pick::Default => "default".into(),
        Pick::One(v) => v.target.clone(),
        Pick::Clash(t) => format!("a clash ({})", t.join(", ")),
    }
}

/// This machine's targets; `None` when it is not registered (or retired).
pub fn this_targets(store: &Store) -> Result<Option<Vec<String>>> {
    let Some(name) = store.local_machine_name()? else {
        return Ok(None);
    };
    let registry = repo::read_registry(&store.repo())?;
    Ok(registry
        .iter()
        .find(|m| m.name == name && !m.retired)
        .map(assign::machine_targets))
}

/// What one variant file receives on a machine, for status and the TUI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Summary {
    pub path: String,
    /// Targets with a variant, besides the default.
    pub targets: Vec<String>,
    /// `default` or the target of the variant received; `None` on a clash.
    pub receives: Option<String>,
    pub clash: Option<String>,
}

pub fn summaries(v: &Variants, targets: &[String]) -> Vec<Summary> {
    v.0.iter()
        .map(|(path, entries)| {
            let picked = pick(entries, targets);
            Summary {
                path: path.clone(),
                targets: entries.iter().map(|e| e.target.clone()).collect(),
                receives: match &picked {
                    Pick::Clash(_) => None,
                    p => Some(describe(p)),
                },
                clash: match picked {
                    Pick::Clash(t) => Some(format!(
                        "ambiguous variants at the same specificity: {}",
                        t.join(", ")
                    )),
                    _ => None,
                },
            }
        })
        .collect()
}

/// Paths whose selection clashes on this machine, with the reason.
pub fn clashes(store: &Store) -> Result<BTreeMap<String, String>> {
    let Some(targets) = this_targets(store)? else {
        return Ok(BTreeMap::new());
    };
    Ok(summaries(&Variants::read(&store.repo())?, &targets)
        .into_iter()
        .filter_map(|s| Some((s.path, s.clash?)))
        .collect())
}

/// What is left to render when chezmoi cannot render a clashed file: every
/// managed file but those (absolute paths). `None` when nothing clashes and
/// chezmoi may be asked about everything.
pub fn render_scope(store: &Store) -> Result<Option<Vec<String>>> {
    let clashed = clashes(store)?;
    if clashed.is_empty() {
        return Ok(None);
    }
    let out = store.chezmoi(&[
        "managed",
        "--include=files,symlinks",
        "--path-style=absolute",
    ])?;
    let home = store.dirs.home.to_string_lossy().into_owned();
    Ok(Some(
        out.stdout
            .lines()
            .filter(|l| {
                l.strip_prefix(&home)
                    .and_then(|r| r.strip_prefix('/'))
                    .is_none_or(|rel| !clashed.contains_key(rel))
            })
            .map(str::to_string)
            .collect(),
    ))
}

/// The template a variant file is stored as. A variant file holding references
/// (it contains `values::MARKER`) is rendered as a template; any other is
/// included raw.
pub(crate) fn template(path: &str) -> String {
    let q = |s: &str| serde_json::to_string(s).expect("a string");
    format!(
        "{{{{- $v := includeTemplate \"dotwarden/variant\" (dict \"data\" . \"path\" {p}) | trim -}}}}\n\
         {{{{- $f := printf \".dotwarden/variants/%s\" (default {d} $v) -}}}}\n\
         {{{{- $t := include $f -}}}}\n\
         {{{{- if contains {m} $t }}}}{{{{ includeTemplate $f $ }}}}{{{{ else }}}}{{{{ $t }}}}{{{{ end -}}}}\n",
        p = q(path),
        d = q(&default_file(path)),
        m = q(crate::values::MARKER),
    )
}

fn slug(target: &str) -> String {
    target.replace(':', "-")
}

fn unique_file(repo: &Path, path: &str, target: &str) -> String {
    let base = format!("{path}/{}", slug(target));
    let mut file = base.clone();
    let mut n = 2;
    while repo.join(repo_path(&file)).exists() {
        file = format!("{base}-{n}");
        n += 1;
    }
    file
}

fn tilde(rel: &str) -> String {
    format!("~/{rel}")
}

fn change(action: &str, path: &str, source: &str) -> FileChange {
    FileChange {
        action: action.into(),
        target: path.into(),
        source: source.into(),
        kind: "file".into(),
    }
}

/// What registry machines receive before and after a change of `path`'s variants.
fn effects(registry: &[Machine], path: &str, before: &[Variant], after: &[Variant]) -> Vec<String> {
    registry
        .iter()
        .filter(|m| !m.retired)
        .filter_map(|m| {
            let t = assign::machine_targets(m);
            let (b, a) = (describe(&pick(before, &t)), describe(&pick(after, &t)));
            (a != b).then(|| format!("{}: {} receives {a} instead of {b}", m.name, tilde(path)))
        })
        .collect()
}

fn variant_entry<'a>(ctx: &'a Ctx, hp: &HomePath) -> Result<&'a crate::source::SourceEntry> {
    let Some(e) = ctx.tracked.files.get(&hp.rel) else {
        let what = if ctx.tracked.under(&hp.rel).next().is_some() {
            "is a folder; variants belong to one file"
        } else {
            "is not tracked"
        };
        return Err(refuse(format!("{} {what}", tilde(&hp.rel))));
    };
    Ok(e)
}

/// Plan a variant of the tracked file `hp` for `target` from its current HOME content.
pub fn plan_add(
    store: &Store,
    ctx: &Ctx,
    hp: &HomePath,
    target: &str,
    requester: &str,
) -> Result<Pending> {
    let repo = store.repo();
    let registry = repo::read_registry(&repo)?;
    let mut notes = Vec::new();
    if let Some(w) = Known::from_registry(&registry).check(target)? {
        notes.push(format!("warning: {w}"));
    }
    let entry = variant_entry(ctx, hp)?;
    let path = hp.rel.as_str();
    if entry.symlink {
        return Err(refuse(format!(
            "{} is a symlink; variants hold file content",
            tilde(path)
        )));
    }
    let vars = Variants::read(&repo)?;
    let existing = vars.get(path);
    let first = existing.is_empty();
    if first && entry.source.ends_with(".tmpl") {
        return Err(refuse(format!("{} is a template already", tilde(path))));
    }
    if !first && !repo.join(repo_path(&default_file(path))).is_file() {
        return Err(refuse(format!(
            "{} keeps its default content inside the template; move it to {} first",
            tilde(path),
            repo_path(&default_file(path))
        )));
    }
    if existing.iter().any(|v| v.target == target) {
        return Err(refuse(format!(
            "{} already has a variant for {target}; remove it first",
            tilde(path)
        )));
    }
    let meta = fs::symlink_metadata(&hp.abs)
        .map_err(|_| refuse(format!("{} does not exist in HOME", tilde(path))))?;
    if !meta.is_file() {
        return Err(refuse(format!("{} is not a regular file", tilde(path))));
    }
    let file = unique_file(&repo, path, target);
    let mut files = Vec::new();
    let mut data = Vec::new();
    if first {
        files.push(change("remove", path, &entry.source));
        files.push(change("add", path, &format!("{}.tmpl", entry.source)));
        files.push(change("add", path, &repo_path(&default_file(path))));
        notes.push(format!(
            "the stored content of {} becomes its default variant",
            tilde(path)
        ));
    }
    files.push(change("add", path, &repo_path(&file)));
    data.push(format!(
        "add variant {target} → {file} for {} in {DATA_FILE}",
        tilde(path)
    ));
    notes.push(format!(
        "the content of {} in HOME becomes the variant for {target}",
        tilde(path)
    ));
    let mut after = existing.to_vec();
    after.push(Variant {
        target: target.into(),
        file: file.clone(),
    });
    let effects = effects(&registry, path, existing, &after);
    let preview = Preview {
        files,
        data,
        notes,
        effects,
        ..Preview::default()
    };
    let summary = format!("Add variant {target} for {}", tilde(path));
    let action = Action::VariantAdd {
        path: path.into(),
        target: target.into(),
        file,
        first,
        source: entry.source.clone(),
    };
    new_item(store, requester, action, summary, preview, ctx)
}

/// Plan removing the variant of `hp` for `target`.
pub fn plan_remove(
    store: &Store,
    ctx: &Ctx,
    hp: &HomePath,
    target: &str,
    requester: &str,
) -> Result<Pending> {
    let repo = store.repo();
    assign::validate_target(target)?;
    let entry = variant_entry(ctx, hp)?;
    let path = hp.rel.as_str();
    let vars = Variants::read(&repo)?;
    let existing = vars.get(path);
    let Some(v) = existing.iter().find(|v| v.target == target) else {
        return Err(refuse(format!(
            "{} has no variant for {target}",
            tilde(path)
        )));
    };
    let last = existing.len() == 1;
    let default = repo_path(&default_file(path));
    if last && !repo.join(&default).is_file() {
        return Err(refuse(format!(
            "{} keeps its default content inside the template; make it a plain file by hand",
            tilde(path)
        )));
    }
    let mut files = Vec::new();
    if repo.join(repo_path(&v.file)).is_file() {
        files.push(change("remove", path, &repo_path(&v.file)));
    }
    let mut notes = Vec::new();
    if last {
        let plain = entry.source.strip_suffix(".tmpl").unwrap_or(&entry.source);
        files.push(change("remove", path, &entry.source));
        files.push(change("remove", path, &default));
        files.push(change("add", path, plain));
        notes.push(format!(
            "the last variant: {} becomes a plain file with its default content",
            tilde(path)
        ));
    }
    let registry = repo::read_registry(&repo)?;
    let after: Vec<Variant> = existing
        .iter()
        .filter(|e| e.target != target)
        .cloned()
        .collect();
    let preview = Preview {
        files,
        data: vec![format!(
            "remove variant {target} of {} from {DATA_FILE}",
            tilde(path)
        )],
        notes,
        effects: effects(&registry, path, existing, &after),
        ..Preview::default()
    };
    let action = Action::VariantRemove {
        path: path.into(),
        target: target.into(),
        file: v.file.clone(),
        last,
        source: entry.source.clone(),
    };
    new_item(
        store,
        requester,
        action,
        format!("Remove variant {target} of {}", tilde(path)),
        preview,
        ctx,
    )
}

/// The repository side of the guard of variant items.
pub fn guard(store: &Store, action: &Action) -> Result<BTreeMap<String, String>> {
    let repo = store.repo();
    let vars = Variants::read(&repo)?;
    let mut g = BTreeMap::new();
    if let Action::VariantAdd { path, source, .. } | Action::VariantRemove { path, source, .. } =
        action
    {
        g.insert(format!("variants:{path}"), vars.fingerprint(path));
        g.insert(
            format!("tracked:{path}"),
            track::repo_fingerprint(&repo, source),
        );
    }
    Ok(g)
}

fn prune(repo: &Path, file: &Path) {
    let stop = repo.join("home");
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == stop || !d.starts_with(&stop) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn remove(repo: &Path, rel: &str) -> Result<()> {
    let p = repo.join(rel);
    if p.is_file() {
        fs::remove_file(&p).with_context(|| format!("removing {}", p.display()))?;
        prune(repo, &p);
    }
    Ok(())
}

fn variants_table(doc: &mut toml_edit::DocumentMut) -> Result<&mut Table> {
    repo::dotwarden_table(doc)?
        .entry("variants")
        .or_insert_with(|| {
            let mut t = Table::new();
            t.set_implicit(true);
            Item::Table(t)
        })
        .as_table_mut()
        .context("`dotwarden.variants` is not a table")
}

/// Make the repository change of a variant item; returns the repository files
/// that must change (the data file also does).
pub fn exec(store: &Store, action: &Action) -> Result<BTreeSet<String>> {
    let repo = store.repo();
    let mut expected = BTreeSet::new();
    match action {
        Action::VariantAdd {
            path,
            target,
            file,
            first,
            source,
        } => {
            let content = fs::read(store.dirs.home.join(path))
                .with_context(|| format!("reading {} in HOME", tilde(path)))?;
            if *first {
                let old = repo.join(source);
                let default =
                    fs::read(&old).with_context(|| format!("reading {}", old.display()))?;
                let (dp, tp) = (repo_path(&default_file(path)), format!("{source}.tmpl"));
                write(&repo.join(&dp), default)?;
                write(&repo.join(&tp), template(path))?;
                fs::remove_file(&old)?;
                expected.extend([dp, tp, source.clone()]);
            }
            write(&repo.join(repo_path(file)), content)?;
            expected.insert(repo_path(file));
            let (p, t, f) = (path.clone(), target.clone(), file.clone());
            repo::edit_data(&repo, |doc| {
                let list = variants_table(doc)?
                    .entry(&p)
                    .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
                    .as_array_of_tables_mut()
                    .context("variants of this path are not a list of tables")?;
                let mut e = Table::new();
                e["target"] = value(t);
                e["file"] = value(f);
                list.push(e);
                Ok(())
            })?;
        }
        Action::VariantRemove {
            path,
            target,
            file,
            last,
            source,
        } => {
            drop_entry(&repo, path, Some(target))?;
            remove(&repo, &repo_path(file))?;
            expected.insert(repo_path(file));
            if *last {
                let dp = repo_path(&default_file(path));
                let default = fs::read(repo.join(&dp)).with_context(|| format!("reading {dp}"))?;
                let plain = source.strip_suffix(".tmpl").unwrap_or(source);
                write(&repo.join(plain), default)?;
                remove(&repo, source)?;
                remove(&repo, &dp)?;
                expected.extend([dp, plain.to_string(), source.clone()]);
            }
        }
        _ => unreachable!("not a variant action"),
    }
    Ok(expected)
}

/// Remove one variant entry (`Some(target)`) or all of `path`.
fn drop_entry(repo: &Path, path: &str, target: Option<&str>) -> Result<()> {
    repo::edit_data(repo, |doc| {
        let table = variants_table(doc)?;
        if let Some(list) = table.get_mut(path).and_then(Item::as_array_of_tables_mut)
            && let Some(t) = target
        {
            let at = list
                .iter()
                .position(|e| e.get("target").and_then(Item::as_str) == Some(t));
            if let Some(i) = at {
                list.remove(i);
            }
            if !list.is_empty() {
                return Ok(());
            }
        }
        table.remove(path);
        Ok(())
    })
}

/// Untrack: remove every variant (data and files) of these paths.
pub fn remove_all(store: &Store, paths: &[String]) -> Result<()> {
    let repo = store.repo();
    let vars = Variants::read(&repo)?;
    for p in paths {
        for f in vars.files_of(&repo, p) {
            remove(&repo, &f)?;
        }
        drop_entry(&repo, p, None)?;
    }
    Ok(())
}

/// After a variant was added: when this machine now receives it, HOME holds
/// exactly what it received, so chezmoi's last-written record follows.
pub fn after_add(store: &Store, path: &str, target: &str) -> Result<()> {
    let Some(mine) = this_targets(store)? else {
        return Ok(());
    };
    let vars = Variants::read(&store.repo())?;
    if matches!(pick(vars.get(path), &mine), Pick::One(v) if v.target == target) {
        crate::changes::restore_last_written(store, &BTreeSet::from([path]))?;
    }
    Ok(())
}

/// The listing of `dw variant list`: the variants of one path or of all.
pub fn list(store: &Store, only: Option<&str>) -> Result<Vec<Summary>> {
    let vars = Variants::read(&store.repo())?;
    let mine = this_targets(store)?.unwrap_or_default();
    Ok(summaries(&vars, &mine)
        .into_iter()
        .filter(|s| only.is_none_or(|p| s.path == p))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(target: &str) -> Variant {
        Variant {
            target: target.into(),
            file: format!("x/{target}"),
        }
    }

    fn targets(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_most_specific_level_wins_and_ties_clash_only_at_the_best_level() {
        let mine = targets(&[
            "platform:linux",
            "distribution:arch",
            "family:a",
            "family:b",
            "machine:m",
        ]);
        let all = [v("platform:linux"), v("family:a"), v("family:b")];
        assert_eq!(pick(&all, &mine), Pick::Clash(vec!["family:a", "family:b"]));
        let with_machine = [v("family:a"), v("family:b"), v("machine:m")];
        assert_eq!(pick(&with_machine, &mine), Pick::One(&with_machine[2]));
        assert_eq!(pick(&[v("platform:darwin")], &mine), Pick::Default);
        assert_eq!(pick(&[v("weird:x")], &targets(&["weird:x"])), Pick::Default);
    }

    #[test]
    fn variant_files_map_back_to_their_tracked_path() {
        let vars = Variants(BTreeMap::from([(
            ".config/a".to_string(),
            vec![Variant {
                target: "machine:m".into(),
                file: "hand/made".into(),
            }],
        )]));
        let f = |s: &str| format!("{DIR}/{s}");
        assert_eq!(
            vars.target_of(&f("hand/made")).as_deref(),
            Some(".config/a")
        );
        assert_eq!(
            vars.target_of(&f(".config/a/default")).as_deref(),
            Some(".config/a")
        );
        assert_eq!(vars.target_of(&f(".x/machine-m")).as_deref(), Some(".x"));
        assert_eq!(vars.target_of("home/dot_x"), None);
    }
}
