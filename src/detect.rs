//! Secret detection (spec §7): after track and stage, a scanner (known key
//! patterns, secret-looking assignments, high-entropy tokens) and exact matches
//! with existing secrets propose moving the value to the vault and replacing
//! it by a reference. Proposals are pending items; they never contain the
//! value, only where it is (file, line, byte range), and are re-verified when
//! applied. A commit is refused when staged files hold a high-confidence hit.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;

use crate::pending::{Action, FileChange, Item, Preview};
use crate::store::Store;
use crate::track::{self, Ctx, new_item, refuse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// 0-based line.
    pub line: usize,
    /// Byte range of the value within the line.
    pub start: usize,
    pub end: usize,
    /// A suggestion for the secret's name.
    pub hint: String,
    /// Known key format or private key: precise enough to refuse a commit.
    pub strong: bool,
    /// Only an entropy guess (never used to redact displayed text).
    pub generic: bool,
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        [
            (r"AKIA[0-9A-Z]{16}", "AWS_ACCESS_KEY_ID"),
            (r"gh[pousr]_[A-Za-z0-9]{30,}", "GITHUB_TOKEN"),
            (r"github_pat_[A-Za-z0-9_]{40,}", "GITHUB_TOKEN"),
            (r"sk-[A-Za-z0-9_-]{24,}", "API_KEY"),
            (r"xox[baprs]-[A-Za-z0-9-]{10,}", "SLACK_TOKEN"),
            (r"AIza[0-9A-Za-z_-]{35}", "GOOGLE_API_KEY"),
        ]
        .iter()
        .map(|(r, n)| (Regex::new(r).expect("valid pattern"), *n))
        .collect()
    })
}

fn assignment() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"(?i)\b([a-z0-9_.-]*(?:password|passwd|secret|token|api[_-]?key|credential|private[_-]?key)[a-z0-9_.-]*)\s*[=:]\s*["']?([^\s"'#]{8,})"#,
        )
        .expect("valid pattern")
    })
}

fn token() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[A-Za-z0-9+/_=-]{28,}").expect("valid pattern"))
}

fn entropy(s: &str) -> f64 {
    let mut counts: BTreeMap<char, f64> = BTreeMap::new();
    for c in s.chars() {
        *counts.entry(c).or_default() += 1.0;
    }
    let n = s.chars().count() as f64;
    -counts
        .values()
        .map(|c| (c / n) * (c / n).log2())
        .sum::<f64>()
}

fn mixed(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && s.chars().any(|c| c.is_ascii_alphabetic())
}

fn hint_from_key(key: &str) -> String {
    let h: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let h = h.trim_matches('_').to_string();
    if h.is_empty() { "SECRET".into() } else { h }
}

/// True for a line that already uses references or looks like code, not data.
fn skip_line(line: &str) -> bool {
    line.contains("includeTemplate") || line.len() > 2000
}

pub fn is_private_key(text: &str) -> bool {
    text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----")
}

/// Candidate secrets in `text`, at most one per line.
pub fn scan_text(text: &str) -> Vec<Hit> {
    if text.len() > 1 << 20 || text.contains('\0') {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if skip_line(line) {
            continue;
        }
        let known = patterns()
            .iter()
            .find_map(|(re, hint)| re.find(line).map(|m| (m, *hint)));
        if let Some((m, hint)) = known {
            hits.push(Hit {
                line: i,
                start: m.start(),
                end: m.end(),
                hint: hint.into(),
                strong: true,
                generic: false,
            });
            continue;
        }
        if let Some(c) = assignment().captures(line) {
            let (key, v) = (c.get(1).expect("group"), c.get(2).expect("group"));
            let value = v.as_str();
            let placeholder =
                value.starts_with("$") || value.starts_with('{') || value.starts_with('<');
            if !placeholder
                && (entropy(value) >= 3.0 || value.len() >= 16)
                && !value.contains("://")
            {
                hits.push(Hit {
                    line: i,
                    start: v.start(),
                    end: v.end(),
                    hint: hint_from_key(key.as_str()),
                    strong: false,
                    generic: false,
                });
                continue;
            }
        }
        if let Some(m) = token()
            .find_iter(line)
            .find(|m| mixed(m.as_str()) && entropy(m.as_str()) >= 4.3 && !m.as_str().contains('/'))
        {
            hits.push(Hit {
                line: i,
                start: m.start(),
                end: m.end(),
                hint: "SECRET".into(),
                strong: false,
                generic: true,
            });
        }
    }
    hits
}

fn value_at(text: &str, h: &Hit) -> Option<String> {
    text.lines()
        .nth(h.line)
        .and_then(|l| l.get(h.start..h.end))
        .map(str::to_string)
}

fn free_name(taken: &BTreeSet<String>, hint: &str) -> String {
    let base: String = hint.chars().take(56).collect();
    if !taken.iter().any(|t| t.eq_ignore_ascii_case(&base)) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken.iter().any(|t| t.eq_ignore_ascii_case(c)))
        .expect("unbounded")
}

fn taken_names(store: &Store) -> BTreeSet<String> {
    let repo = store.repo();
    let mut t: BTreeSet<String> = crate::secrets::read_meta(&repo).into_keys().collect();
    t.extend(
        crate::values::read_all(&repo)
            .unwrap_or_default()
            .into_keys(),
    );
    t
}

fn tilde(rel: &str) -> String {
    format!("~/{rel}")
}

/// Pending proposals for the secrets found in `text` (the HOME content of
/// `rel`). Needs a vault; without one nothing is proposed.
pub fn moves(store: &Store, ctx: &Ctx, rel: &str, text: &str, requester: &str) -> Vec<Item> {
    if !crate::vault::exists(store) {
        return Vec::new();
    }
    let Some(entry) = ctx.tracked.files.get(rel) else {
        return Vec::new();
    };
    if entry.symlink {
        return Vec::new();
    }
    let mut taken = taken_names(store);
    let mut seen_values = BTreeSet::new();
    let mut items = Vec::new();
    if is_private_key(text)
        && let Ok(hp) = track::resolve_home_path(store, &format!("~/{rel}"), &store.dirs.home)
        && let Ok(item) = crate::encfile::plan(store, ctx, &hp, requester)
    {
        items.push(item);
        return items;
    }
    // Only values that sit in the stored file as plain text (not behind a
    // reference) and are not already a secret of the vault.
    let stored = fs::read_to_string(store.repo().join(&entry.source)).unwrap_or_default();
    let known = known_values(store);
    for h in scan_text(text) {
        let Some(value) = value_at(text, &h) else {
            continue;
        };
        if !stored.contains(&value) || known.contains(&value) {
            continue;
        }
        if !seen_values.insert(value) {
            continue;
        }
        let name = free_name(&taken, &h.hint);
        if let Ok(item) = plan_move(store, ctx, rel, &h, &name, requester) {
            taken.insert(name);
            items.push(item);
        }
    }
    items
}

pub fn plan_move(
    store: &Store,
    ctx: &Ctx,
    rel: &str,
    h: &Hit,
    name: &str,
    requester: &str,
) -> Result<Item> {
    crate::values::check_name(name)?;
    let entry = ctx
        .tracked
        .files
        .get(rel)
        .ok_or_else(|| refuse(format!("{} is not a tracked file", tilde(rel))))?;
    let action = Action::SecretMove {
        key: format!("{rel}@{}:{}-{}", h.line + 1, h.start, h.end),
        path: rel.into(),
        name: name.into(),
        line: h.line,
        start: h.start,
        end: h.end,
    };
    let change = |a: &str, src: &str| FileChange {
        action: a.into(),
        target: rel.into(),
        source: src.into(),
        kind: "file".into(),
    };
    let preview = Preview {
        files: vec![
            change("change", &entry.source),
            change("add", &crate::secrets::file_of(name)),
            change("add", &crate::values::file_of(name)),
        ],
        notes: vec![
            format!(
                "line {} of {} holds a value that looks like a secret (hidden); it becomes secret {name} in the vault and the file keeps a reference",
                h.line + 1,
                tilde(rel)
            ),
            "the value is encrypted to the vault recipient (no unlock needed); HOME is not changed"
                .into(),
        ],
        ..Preview::default()
    };
    new_item(
        store,
        requester,
        action,
        format!(
            "Move a secret of {} (line {}) to the vault as {name}",
            tilde(rel),
            h.line + 1
        ),
        preview,
        ctx,
    )
}

pub fn guard(store: &Store, ctx: &Ctx, action: &Action) -> BTreeMap<String, String> {
    let repo = store.repo();
    let mut g = BTreeMap::new();
    match action {
        Action::SecretMove { path, name, .. } => {
            g.insert(
                format!("value:{name}"),
                track::repo_fingerprint(&repo, &crate::values::file_of(name)),
            );
            if let Some(e) = ctx.tracked.files.get(path) {
                g.insert(
                    format!("tracked:{path}"),
                    track::repo_fingerprint(&repo, &e.source),
                );
            }
        }
        Action::Encrypt { path, .. } => {
            if let Some(e) = ctx.tracked.files.get(path) {
                g.insert(
                    format!("tracked:{path}"),
                    track::repo_fingerprint(&repo, &e.source),
                );
            }
        }
        _ => {}
    }
    g
}

/// Make the repository change of a `SecretMove`; returns the files that change.
pub fn exec(store: &Store, action: &Action) -> Result<BTreeSet<String>> {
    let Action::SecretMove {
        path,
        name,
        line,
        start,
        end,
        ..
    } = action
    else {
        unreachable!("not a secret move");
    };
    let text = fs::read_to_string(store.dirs.home.join(path))
        .with_context(|| format!("reading ~/{path}"))?;
    let hit = Hit {
        line: *line,
        start: *start,
        end: *end,
        hint: String::new(),
        strong: false,
        generic: false,
    };
    let value = value_at(&text, &hit)
        .filter(|_| {
            scan_text(&text)
                .iter()
                .any(|h| (h.line, h.start, h.end) == (*line, *start, *end))
        })
        .ok_or_else(|| {
            refuse(format!(
                "~/{path} changed since the proposal; the secret is no longer where it was"
            ))
        })?;
    let repo = store.repo();
    let mut files = BTreeSet::new();
    for (rel, bytes) in crate::secrets::prepare(store, name, None, &value, None)? {
        let p = repo.join(&rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, bytes)?;
        files.insert(rel);
    }
    files.extend(crate::values::wire_in_repo(
        store,
        path,
        name,
        Some(&value),
    )?);
    Ok(files)
}

/// Remove unreachable git objects: the plaintext blob a track or stage added
/// before the value moved to the vault.
pub fn prune(store: &Store) -> Result<()> {
    let repo = store.repo();
    let git = |args: &[&str]| -> Result<()> {
        let mut cmd = store.git_command(&repo);
        cmd.args(args);
        crate::exec::run_ok(cmd, None)?;
        Ok(())
    };
    git(&["reflog", "expire", "--expire=now", "--all"])?;
    git(&["prune", "--expire=now"])
}

/// Refuse a commit that would record a key-like value, a private key or the
/// value of an existing secret (`DOTWARDEN_ALLOW_SECRETS=1`, humans only, accepts false positives); never prints the value.
pub fn commit_guard(store: &Store) -> Result<()> {
    // A human can accept false positives for one commit; an agent cannot.
    if crate::changes::agent(store).is_none()
        && store
            .env()
            .contains_key(std::ffi::OsStr::new("DOTWARDEN_ALLOW_SECRETS"))
    {
        return Ok(());
    }
    let repo = store.repo();
    let names = crate::exec::run_ok(
        {
            let mut c = store.git_command(&repo);
            c.args(["diff", "--cached", "--name-only", "--diff-filter=AM", "-z"]);
            c
        },
        None,
    )?
    .stdout;
    let known: Vec<String> = known_values(store);
    let mut bad = Vec::new();
    for rel in names.split('\0').filter(|n| !n.is_empty()) {
        if rel.starts_with(crate::vault::ROOT) || rel.starts_with(crate::values::DIR) {
            continue;
        }
        let Ok(o) = crate::exec::run_ok(
            {
                let mut c = store.git_command(&repo);
                c.args(["show", &format!(":{rel}")]);
                c
            },
            None,
        ) else {
            continue;
        };
        let t = &o.stdout;
        if is_private_key(t)
            || scan_text(t).iter().any(|h| !h.generic)
            || known.iter().any(|v| t.contains(v.as_str()))
        {
            bad.push(rel.to_string());
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    Err(refuse(format!(
        "staged files appear to contain secrets ({}); move them to the vault (see `dw pending`) or encrypt the file (`dw encrypt`)",
        bad.join(", ")
    )))
}

/// Values of existing secrets this machine receives (empty while locked).
fn known_values(store: &Store) -> Vec<String> {
    let Ok(r) = crate::values::Resolver::load(store) else {
        return Vec::new();
    };
    r.secrets
        .keys()
        .filter_map(|n| r.value(n).ok())
        .filter(|v| v.chars().count() >= 3)
        .collect()
}

/// `text` with every known secret value and every precise-looking secret
/// replaced by a marker. Used on anything displayed that came from files.
pub fn redact(text: &str, known: &[(String, String)]) -> String {
    let mut t = text.to_string();
    let mut by_len: Vec<_> = known.iter().filter(|(_, v)| !v.is_empty()).collect();
    by_len.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    for (n, v) in by_len {
        t = t.replace(v.as_str(), &crate::secrets::marker(n));
    }
    let mut out = Vec::new();
    let mut in_key = false;
    let hits: BTreeMap<usize, Hit> = scan_text(&t)
        .into_iter()
        .filter(|h| !h.generic)
        .map(|h| (h.line, h))
        .collect();
    for (i, line) in t.lines().enumerate() {
        if line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----") {
            in_key = true;
        }
        if in_key {
            if line.contains("-----END ") {
                in_key = false;
            }
            out.push("«private key hidden»".to_string());
            continue;
        }
        match hits.get(&i) {
            Some(h) if line.get(h.start..h.end).is_some() => {
                out.push(format!(
                    "{}«possible secret»{}",
                    &line[..h.start],
                    &line[h.end..]
                ));
            }
            _ => out.push(line.to_string()),
        }
    }
    let mut s = out.join("\n");
    if t.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// The secrets whose values this machine can read now, as (name, value).
pub fn known_pairs(store: &Store) -> Vec<(String, String)> {
    let Ok(r) = crate::values::Resolver::load(store) else {
        return Vec::new();
    };
    r.secrets
        .keys()
        .filter_map(|n| Some((n.clone(), r.value(n).ok()?)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}
