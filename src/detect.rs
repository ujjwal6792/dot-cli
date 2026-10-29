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

// ---- never write detected secrets into the repository -----------------

static ALLOW_PLAINTEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--allow-plaintext` (humans only): track and stage files as they are.
pub fn set_allow_plaintext(v: bool) {
    ALLOW_PLAINTEXT.store(v, std::sync::atomic::Ordering::SeqCst);
}

/// A HOME file whose content holds secrets: `values` are (value, existing
/// secret name if the vault already has it, name hint).
#[derive(Debug, Clone)]
pub struct Found {
    pub rel: String,
    pub private_key: bool,
    pub values: Vec<(String, Option<String>, String)>,
}

/// The files among `rels` that hold secrets. Without a vault (and without
/// `--allow-plaintext`) this refuses, so nothing is written anywhere; with one
/// the caller stores them sanitized. Only file names are ever reported.
pub fn inspect(store: &Store, rels: &[String]) -> Result<Vec<Found>> {
    let out = found_in(store, rels);
    if !out.is_empty() && !crate::vault::exists(store) {
        return Err(refuse(no_vault_message(&out)));
    }
    Ok(out)
}

pub fn no_vault_message(out: &[Found]) -> String {
    let names: Vec<String> = out.iter().map(|f| tilde(&f.rel)).collect();
    format!(
        "{} appear(s) to contain a secret, which must not be written to the repository: \
         run `dw vault init`, then track again — the secret will be moved to the vault \
         (a human can pass --allow-plaintext to store the file as it is)",
        names.join(", ")
    )
}

/// Like [`inspect`] without the vault check.
pub fn found_in(store: &Store, rels: &[String]) -> Vec<Found> {
    if ALLOW_PLAINTEXT.load(std::sync::atomic::Ordering::SeqCst) {
        return Vec::new();
    }
    let known = known_pairs(store);
    let mut out = Vec::new();
    for rel in rels {
        let p = store.dirs.home.join(rel);
        if fs::symlink_metadata(&p).is_ok_and(|m| !m.is_file()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let mut values: Vec<(String, Option<String>, String)> = Vec::new();
        for (n, v) in &known {
            if v.chars().count() >= 3 && text.contains(v.as_str()) {
                values.push((v.clone(), Some(n.clone()), n.clone()));
            }
        }
        for h in scan_text(&text).into_iter().filter(|h| !h.generic) {
            if let Some(v) = value_at(&text, &h)
                && !values.iter().any(|(x, _, _)| *x == v)
            {
                values.push((v, None, h.hint.clone()));
            }
        }
        let private_key = is_private_key(&text);
        if private_key || !values.is_empty() {
            out.push(Found {
                rel: rel.clone(),
                private_key,
                values,
            });
        }
    }
    out
}

/// What to store for a file with secrets: its new source content and the vault
/// files that go with it. The plaintext appears in none of them.
pub struct Sanitized {
    pub bytes: Vec<u8>,
    pub extra: Vec<(String, Vec<u8>)>,
    /// Names of the secrets created or referenced.
    pub names: Vec<String>,
}

pub fn sanitize(store: &Store, f: &Found, taken: &mut BTreeSet<String>) -> Result<Sanitized> {
    let text = fs::read_to_string(store.dirs.home.join(&f.rel))?;
    if f.private_key {
        let raw: [u8; 8] = rand::random();
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        return Ok(Sanitized {
            bytes: crate::render::file_stub(&id).into_bytes(),
            extra: vec![(
                crate::encfile::age_file(&id),
                crate::vault::encrypt(store, text.as_bytes())?,
            )],
            names: Vec::new(),
        });
    }
    let mut segs = vec![crate::values::Seg::Lit(text)];
    let (mut extra, mut names) = (Vec::new(), Vec::new());
    for (value, existing, hint) in &f.values {
        let name = match existing {
            Some(n) => n.clone(),
            None => {
                let n = free_name(taken, hint);
                taken.insert(n.clone());
                extra.extend(crate::secrets::prepare(store, &n, None, value, None)?);
                n
            }
        };
        segs = crate::values::wire_segs(&segs, &name, value).0;
        names.push(name);
    }
    Ok(Sanitized {
        bytes: crate::values::to_template(&segs).into_bytes(),
        extra,
        names,
    })
}

pub fn taken(store: &Store) -> BTreeSet<String> {
    taken_names(store)
}

pub fn write_extra(store: &Store, extra: &[(String, Vec<u8>)]) -> Result<()> {
    let repo = store.repo();
    for (rel, bytes) in extra {
        let p = repo.join(rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, bytes)?;
    }
    Ok(())
}

/// Add a new HOME file to the source state with `bytes` as its (template)
/// content: chezmoi adds a sanitized copy in a throwaway destination, so the
/// plaintext never reaches the repository; its last-written record is then set
/// from HOME itself (the template renders to exactly that).
pub fn add_sanitized(store: &Store, rel: &str, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let raw: [u8; 8] = rand::random();
    let name: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = store
        .dirs
        .state_dir()
        .join("run")
        .join(format!("add-{name}"));
    let dest = tmp.join("dest");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dest)?;
    let mut dir = dest.clone();
    let mut src = store.dirs.home.clone();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        dir = dir.join(part);
        src = src.join(part);
        let mode = fs::metadata(&src).map_or(0o755, |m| m.permissions().mode() & 0o777);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(mode)
            .create(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode))?;
    }
    let result = (|| -> Result<()> {
        let file = dest.join(rel);
        fs::write(&file, bytes)?;
        let mode = fs::metadata(store.dirs.home.join(rel))?
            .permissions()
            .mode()
            & 0o777;
        fs::set_permissions(&file, fs::Permissions::from_mode(mode))?;
        let _g = store.chezmoi_guard()?;
        let mut cmd = store.chezmoi_command_with(
            &store.dirs.chezmoi_config(),
            &tmp.join("state.db"),
            &tmp.join("cache"),
        );
        cmd.arg("--destination").arg(&dest);
        cmd.args(["add", "--template"]).arg(&file);
        crate::exec::run_ok(cmd, None)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    result?;
    set_last_written(store, rel)
}

/// Record HOME's current content as what chezmoi last wrote for `rel`.
pub fn set_last_written(store: &Store, rel: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let p = store.dirs.home.join(rel);
    let data = fs::read(&p)?;
    let sha: String = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mode = fs::metadata(&p)?.permissions().mode() & 0o777;
    let key = p.to_string_lossy().into_owned();
    let value = serde_json::json!({"type": "file", "mode": mode, "contentsSHA256": sha});
    store.chezmoi_locked(&[
        "state".to_string(),
        "set".into(),
        "--bucket=entryState".into(),
        format!("--key={key}"),
        format!("--value={value}"),
    ])?;
    Ok(())
}
