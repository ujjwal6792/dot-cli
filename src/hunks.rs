//! Hunk and line staging (spec §5): stage or unstage selected parts of one
//! file's diff. Staging writes the committed text plus the selected changes into
//! the source file; unstaging writes the staged text minus the selected changes.
//! HOME is never touched. Only plain text files are editable: a file with
//! references, a template, a variant, a secret or a new file is shown masked and
//! read-only, and is staged whole.

use std::collections::{BTreeSet, HashMap};
use std::fs;

use anyhow::Result;
use serde::Serialize;
use similar::{ChangeTag, TextDiff};

use crate::changes::{self, State};
use crate::error::{DwError, Kind};
use crate::store::Store;
use crate::track::{self, Ctx, git_literal, refuse};

const CONTEXT: usize = 3;
const HIDDEN: &str = "«hidden: possible secret»";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// HOME against what is committed or staged: stage selected changes.
    Stage,
    /// The index against HEAD: unstage selected changes.
    Unstage,
}

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    /// `' '` context, `'+'` added, `'-'` removed.
    pub tag: char,
    pub text: String,
    /// Number of a changed line (1-based, over the whole file); `None` for context.
    pub id: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hunk {
    /// 1-based.
    pub number: usize,
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub path: String,
    pub side: Side,
    pub hunks: Vec<Hunk>,
    /// Why this file cannot be staged by hunk, when it cannot.
    pub readonly: Option<String>,
    /// Identifies the diff shown; a stale selection is refused.
    pub fingerprint: String,
}

impl View {
    pub fn change_lines(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.id.is_some())
            .count()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub hunks: Vec<usize>,
    pub lines: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub path: String,
    pub side: Side,
    /// Changed lines moved.
    pub lines: usize,
    pub dry_run: bool,
}

fn fingerprint(old: &str, new: &str) -> String {
    changes::sha256(format!("{old}\0{new}").as_bytes())
}

fn ids(old: &str, new: &str) -> HashMap<(char, usize), usize> {
    let diff = TextDiff::from_lines(old, new);
    let mut next = 0;
    let mut map = HashMap::new();
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Delete => {
                next += 1;
                map.insert(('-', c.old_index().unwrap_or_default()), next);
            }
            ChangeTag::Insert => {
                next += 1;
                map.insert(('+', c.new_index().unwrap_or_default()), next);
            }
            ChangeTag::Equal => {}
        }
    }
    map
}

/// The hunks of `old` → `new`, with every changed line numbered.
pub fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let numbers = ids(old, new);
    let diff = TextDiff::from_lines(old, new);
    diff.grouped_ops(CONTEXT)
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let (first, last) = (&group[0], &group[group.len() - 1]);
            let (a, b) = (first.old_range().start, first.new_range().start);
            let header = format!(
                "@@ -{},{} +{},{} @@",
                a + 1,
                last.old_range().end - a,
                b + 1,
                last.new_range().end - b
            );
            let lines = group
                .iter()
                .flat_map(|op| diff.iter_changes(op))
                .map(|c| {
                    let (tag, id) = match c.tag() {
                        ChangeTag::Equal => (' ', None),
                        ChangeTag::Delete => {
                            ('-', numbers.get(&('-', c.old_index().unwrap_or_default())))
                        }
                        ChangeTag::Insert => {
                            ('+', numbers.get(&('+', c.new_index().unwrap_or_default())))
                        }
                    };
                    Line {
                        tag,
                        text: c.value().trim_end_matches('\n').to_string(),
                        id: id.copied(),
                    }
                })
                .collect();
            Hunk {
                number: i + 1,
                header,
                lines,
            }
        })
        .collect()
}

/// `old` with only the changes numbered in `take` applied.
pub fn merge(old: &str, new: &str, take: &BTreeSet<usize>) -> String {
    let numbers = ids(old, new);
    let mut out = String::new();
    for c in TextDiff::from_lines(old, new).iter_all_changes() {
        let keep = match c.tag() {
            ChangeTag::Equal => true,
            ChangeTag::Delete => {
                !take.contains(&numbers[&('-', c.old_index().unwrap_or_default())])
            }
            ChangeTag::Insert => take.contains(&numbers[&('+', c.new_index().unwrap_or_default())]),
        };
        if keep {
            out.push_str(c.value());
        }
    }
    out
}

struct Ground {
    old: String,
    new: String,
    /// The source file (relative to the repository) that gets the result.
    source: Option<String>,
    readonly: Option<String>,
}

fn masked(g: &mut Ground, reason: impl Into<String>, old: String, new: String) {
    g.old = old;
    g.new = new;
    g.source = None;
    g.readonly = Some(reason.into());
}

fn ground(store: &Store, rel: &str, side: Side) -> Result<Ground> {
    let ctx = Ctx::load(store)?;
    let scan = changes::scan(store, &ctx)?;
    if let Some(why) = scan.clashes.get(rel) {
        return Err(refuse(format!("~/{rel}: {why}")));
    }
    if scan.locked.contains(rel) {
        return Err(DwError::new(
            Kind::VaultLocked,
            format!(
                "~/{rel} needs the vault, which is locked; run `dw vault unlock` in a terminal"
            ),
        )
        .into());
    }
    let wanted = match side {
        Side::Stage => [State::Local, State::New],
        Side::Unstage => [State::Staged, State::Staged],
    };
    let Some(change) = scan
        .changes
        .iter()
        .find(|c| c.kind != "repo" && c.path == rel && wanted.contains(&c.state))
    else {
        let state = scan.changes.iter().find(|c| c.path == rel).map(|c| c.state);
        return Err(refuse(match state {
            Some(State::Incoming) => {
                format!("~/{rel}: an incoming change would be overwritten; apply it first")
            }
            Some(State::Conflict) => {
                format!("~/{rel}: changed here and by an incoming change; resolve it first")
            }
            _ => format!(
                "~/{rel} has nothing to {}",
                if side == Side::Stage {
                    "stage"
                } else {
                    "unstage"
                }
            ),
        }));
    };
    let known = crate::detect::known_pairs(store);
    let hide = |t: &str| crate::detect::redact(t, &known);
    let entry = ctx.tracked.files.get(rel);
    let vars = crate::variants::Variants::read(&store.repo())?;
    let mut g = Ground {
        old: String::new(),
        new: String::new(),
        source: None,
        readonly: None,
    };
    let source_plain = entry.filter(|e| !e.symlink && !e.source.ends_with(".tmpl"));
    match side {
        Side::Stage => {
            let home = store.dirs.home.join(rel);
            let old = scan.targets.get(rel).map(|t| t.text.clone());
            let bytes = fs::read(&home).ok();
            let Some(new) = bytes.as_deref().and_then(|b| std::str::from_utf8(b).ok()) else {
                g.readonly = Some("deleted, a link or not text; stage it whole with Space".into());
                return Ok(g);
            };
            if let Some((o, n)) = crate::values::masked_diff(
                store,
                &ctx,
                &crate::values::Resolver::load(store)?,
                &vars,
                rel,
                new,
            ) {
                masked(
                    &mut g,
                    "it holds references; stage it whole with Space (values go to the vault)",
                    o,
                    n,
                );
                return Ok(g);
            }
            let found = crate::detect::found_in(store, &[rel.to_string()]);
            g.old = hide(old.as_deref().unwrap_or_default());
            g.new = hide(new);
            if let Some(f) = found.first() {
                let spots: BTreeSet<usize> = f.spots.iter().map(|(l, _)| *l).collect();
                g.new = g
                    .new
                    .split_inclusive('\n')
                    .enumerate()
                    .map(|(i, l)| {
                        if spots.contains(&(i + 1)) {
                            format!("{HIDDEN}\n")
                        } else {
                            l.to_string()
                        }
                    })
                    .collect();
                g.readonly = Some(
                    "it holds a secret; stage it whole with Space (it moves to the vault)".into(),
                );
            } else if change.state == State::New || old.is_none() {
                g.readonly = Some("it is new; stage it whole with Space".into());
            } else if vars.0.contains_key(rel) {
                g.readonly = Some("it is a variant file; stage it whole with Space".into());
            } else if source_plain.is_none() {
                g.readonly = Some("it is a template or link; stage it whole with Space".into());
            } else if let Some(e) = source_plain {
                if fs::read_to_string(store.repo().join(&e.source)).ok() == old {
                    g.old = old.unwrap_or_default();
                    g.new = new.to_string();
                    g.source = Some(e.source.clone());
                } else {
                    g.readonly =
                        Some("its source is not a plain copy; stage it whole with Space".into());
                }
            }
        }
        Side::Unstage => {
            let Some(e) = entry else {
                return Err(refuse(format!("~/{rel} is not tracked")));
            };
            let head = git_literal(
                store,
                &["show".as_ref(), format!("HEAD:{}", e.source).as_ref()],
            )
            .ok();
            let index = fs::read_to_string(store.repo().join(&e.source)).ok();
            let (Some(head), Some(index)) = (head, index) else {
                g.readonly =
                    Some("it is new or removed in this commit; unstage it whole with Space".into());
                return Ok(g);
            };
            g.old = hide(&head.stdout);
            g.new = hide(&index);
            if source_plain.is_none() || vars.0.contains_key(rel) {
                g.readonly =
                    Some("it is a template, link or variant; unstage it whole with Space".into());
            } else if g.old != head.stdout || g.new != index {
                g.readonly = Some("it holds a secret value; unstage it whole with Space".into());
            } else {
                g.old = head.stdout;
                g.new = index;
                g.source = Some(e.source.clone());
            }
        }
    }
    Ok(g)
}

/// The staging view of one file: what Stage (Local) or Unstage (Staged) shows.
pub fn open(store: &Store, rel: &str, side: Side) -> Result<View> {
    let g = ground(store, rel, side)?;
    Ok(View {
        path: rel.to_string(),
        side,
        hunks: hunks(&g.old, &g.new),
        readonly: g.readonly,
        fingerprint: fingerprint(&g.old, &g.new),
    })
}

fn usage(msg: String) -> anyhow::Error {
    DwError::new(Kind::Usage, msg).into()
}

/// Stage (or unstage) the selected hunks and lines of `rel`. `expect` is the
/// fingerprint of the view the selection was made on.
pub fn apply(
    store: &Store,
    rel: &str,
    side: Side,
    sel: &Selection,
    expect: Option<&str>,
    dry_run: bool,
) -> Result<Outcome> {
    let lock = (!dry_run).then(|| store.lock()).transpose()?;
    let g = ground(store, rel, side)?;
    if let Some(reason) = &g.readonly {
        return Err(refuse(format!("~/{rel}: {reason}")));
    }
    let shown = hunks(&g.old, &g.new);
    if expect.is_some_and(|e| e != fingerprint(&g.old, &g.new)) {
        return Err(DwError::new(
            Kind::Stale,
            format!("~/{rel} changed since it was shown; open it again"),
        )
        .into());
    }
    let total: usize = shown
        .iter()
        .flat_map(|h| &h.lines)
        .filter(|l| l.id.is_some())
        .count();
    let mut take = BTreeSet::new();
    for n in &sel.hunks {
        let h = shown
            .get(n.wrapping_sub(1))
            .ok_or_else(|| usage(format!("~/{rel} has no hunk {n} ({} hunk(s))", shown.len())))?;
        take.extend(h.lines.iter().filter_map(|l| l.id));
    }
    for n in &sel.lines {
        if *n == 0 || *n > total {
            return Err(usage(format!(
                "~/{rel} has no changed line {n} ({total} changed line(s))"
            )));
        }
        take.insert(*n);
    }
    if take.is_empty() {
        return Err(usage("select at least one hunk or line".into()));
    }
    let out = Outcome {
        path: track::tilde(rel),
        side,
        lines: take.len(),
        dry_run,
    };
    if dry_run {
        return Ok(out);
    }
    let _lock = lock;
    let source = g.source.expect("editable ground has a source");
    let text = match side {
        Side::Stage => merge(&g.old, &g.new, &take),
        Side::Unstage => {
            let keep: BTreeSet<usize> = (1..=total).filter(|i| !take.contains(i)).collect();
            merge(&g.old, &g.new, &keep)
        }
    };
    track::ensure_repo_clean(store)?;
    let tree = track::index_tree(store)?;
    let backup = track::state_backup(store)?;
    let mark = crate::undo::mark(store);
    let run = || -> Result<()> {
        fs::write(store.repo().join(&source), text)?;
        // The committed text moved, so what chezmoi last wrote must move with it.
        changes::restore_last_written(store, &BTreeSet::from([rel]))?;
        let changed: Vec<String> = track::unstaged(store)?.into_iter().collect();
        track::stage(store, &changed)
    };
    if let Err(e) = run() {
        track::rollback(store, &tree)?;
        track::state_restore(store, backup)?;
        return Err(e.context("staging failed; the repository was left as before"));
    }
    let (kind, verb) = match side {
        Side::Stage => ("stage", "stage lines of"),
        Side::Unstage => ("unstage", "unstage lines of"),
    };
    let summary = crate::undo::describe(verb, &[rel.to_string()]);
    crate::undo::record(store, mark, kind, summary, &crate::undo::actor(store));
    Ok(out)
}
