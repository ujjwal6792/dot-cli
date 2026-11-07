//! Commit history: log, show and revert (spec §6). Everything shown is masked
//! with the known secret values; stored files hold references, not values, so
//! this is a second line of defence.

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::error::{DwError, Kind};
use crate::store::Store;

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    /// The `Agent:` trailer, when an agent made the commit.
    pub agent: Option<String>,
    pub merge: bool,
}

const SEP: char = '\u{1f}';
const END: char = '\u{1e}';

/// The repository path for `arg`: a HOME path is mapped to the file it is stored as.
pub fn repo_path(store: &Store, arg: &str) -> String {
    let repo = store.repo();
    if !std::path::Path::new(arg).is_absolute() && repo.join(arg).exists() {
        return arg.to_string();
    }
    if let Ok(rel) = std::path::Path::new(arg).strip_prefix(&repo) {
        return rel.to_string_lossy().into_owned();
    }
    let abs = if let Some(rest) = arg.strip_prefix("~/") {
        store.dirs.home.join(rest)
    } else if std::path::Path::new(arg).is_absolute() {
        arg.into()
    } else {
        store.dirs.home.join(arg)
    };
    store
        .chezmoi(&["source-path".as_ref(), abs.as_os_str()])
        .ok()
        .and_then(|o| {
            let p = o.stdout.trim().to_string();
            p.strip_prefix(&format!("{}/", repo.display()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| arg.to_string())
}

fn repo_paths(store: &Store, arg: &str) -> Result<Vec<String>> {
    let mut paths = std::collections::BTreeSet::from([repo_path(store, arg)]);
    let home = std::path::Path::new(arg)
        .strip_prefix(&store.dirs.home)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| arg.strip_prefix("~/").unwrap_or(arg).to_string());
    // Include old stored spellings (e.g. before conversion to a template),
    // and files that no longer exist in the current tree.
    let names = store.git(&[
        "log",
        "--format=",
        "--name-only",
        "-z",
        "HEAD",
        "--",
        "home",
    ])?;
    let vars = crate::variants::Variants::read(&store.repo()).ok();
    for path in names.stdout.split('\0').map(|p| p.trim_start_matches('\n')) {
        let target = crate::source::decode_source_path(&store.repo(), path)
            .map(|(p, _)| p)
            .or_else(|| vars.as_ref().and_then(|v| v.target_of(path)));
        if target
            .as_deref()
            .is_some_and(|t| t == home || t.starts_with(&format!("{home}/")))
        {
            paths.insert(path.to_string());
        }
    }
    Ok(paths.into_iter().collect())
}

/// `count` commits (first-parent, newest first) after skipping `skip`, optionally
/// only those touching `path`.
pub fn log(store: &Store, path: Option<&str>, skip: usize, count: usize) -> Result<Vec<Commit>> {
    let fmt = format!(
        "--format=%H{SEP}%h{SEP}%an{SEP}%aI{SEP}%s{SEP}%P{SEP}%(trailers:key=Agent,valueonly,separator=%x2c){END}"
    );
    let mut args = vec![
        "--literal-pathspecs".to_string(),
        "log".to_string(),
        "--first-parent".into(),
        format!("--skip={skip}"),
        format!("-n{count}"),
        fmt,
    ];
    if let Some(p) = path {
        args.push("--".into());
        args.extend(repo_paths(store, p)?);
    }
    let out = store.git(&args)?;
    let mut commits = Vec::new();
    for rec in out.stdout.split(END) {
        let rec = rec.trim_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let f: Vec<&str> = rec.split(SEP).collect();
        if f.len() < 7 {
            continue;
        }
        let agent = f[6].trim();
        commits.push(Commit {
            hash: f[0].into(),
            short: f[1].into(),
            author: crate::conflicts::masked(store, f[2]),
            date: f[3].into(),
            subject: crate::conflicts::masked(store, f[4]),
            merge: f[5].split_whitespace().count() > 1,
            agent: (!agent.is_empty()).then(|| crate::conflicts::masked(store, agent)),
        });
    }
    Ok(commits)
}

/// Resolve `commit` to a full hash.
pub fn resolve(store: &Store, commit: &str) -> Result<String> {
    let out = store
        .git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{commit}^{{commit}}"),
        ])
        .map_err(|_| anyhow::anyhow!("{commit} is not a commit"))?;
    Ok(out.stdout.trim().to_string())
}

/// The masked unified diff of a commit (against its first parent).
pub fn diff(store: &Store, commit: &str, path: Option<&str>) -> Result<String> {
    let hash = resolve(store, commit)?;
    let mut args = vec![
        "--literal-pathspecs".to_string(),
        "show".to_string(),
        "--no-color".into(),
        "--first-parent".into(),
        "-m".into(),
        "--format=".into(),
        hash,
    ];
    if let Some(p) = path {
        args.push("--".into());
        args.extend(repo_paths(store, p)?);
    }
    let out = store.git(&args)?;
    Ok(crate::conflicts::masked(store, &out.stdout))
}

/// The files a commit changed (against its first parent) as (shown name, repository path).
pub fn commit_files(store: &Store, commit: &str) -> Result<Vec<(String, String)>> {
    let hash = resolve(store, commit)?;
    let out = store.git(&[
        "show",
        "--first-parent",
        "-m",
        "--format=",
        "--name-only",
        "-z",
        &hash,
    ])?;
    let mut files: Vec<(String, String)> = out
        .stdout
        .split('\0')
        .map(|p| p.trim_start_matches('\n'))
        .filter(|p| !p.is_empty())
        .map(|p| {
            let shown = crate::source::decode_source_path(&store.repo(), p)
                .map(|(t, _)| format!("~/{t}"))
                .unwrap_or_else(|| p.to_string());
            (shown, p.to_string())
        })
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// The masked diff of one repository path of a commit.
pub fn diff_file(store: &Store, commit: &str, repo_path: &str) -> Result<String> {
    let hash = resolve(store, commit)?;
    let out = store.git(&[
        "--literal-pathspecs",
        "show",
        "--no-color",
        "--first-parent",
        "-m",
        "--format=",
        &hash,
        "--",
        repo_path,
    ])?;
    Ok(crate::conflicts::masked(store, &out.stdout))
}

/// Header lines of a commit for `dw show`.
pub fn header(c: &Commit) -> String {
    let mut s = format!(
        "commit {}\nAuthor: {}\nDate:   {}\n",
        c.hash, c.author, c.date
    );
    if let Some(a) = &c.agent {
        s.push_str(&format!("Agent:  {a}\n"));
    }
    s.push_str(&format!("\n    {}\n\n", c.subject));
    s
}

pub fn commit_info(store: &Store, commit: &str) -> Result<Commit> {
    let hash = resolve(store, commit)?;
    let out = store.git(&[
        "log",
        "-n1",
        &format!("--format=%H{SEP}%h{SEP}%an{SEP}%aI{SEP}%s{SEP}%P{SEP}%(trailers:key=Agent,valueonly,separator=%x2c){END}"),
        &hash,
    ])?;
    let f: Vec<&str> = out
        .stdout
        .trim_end_matches(['\n', END])
        .split(SEP)
        .collect();
    if f.len() < 7 {
        bail!("cannot read commit {commit}");
    }
    let agent = f[6].trim();
    Ok(Commit {
        hash: f[0].into(),
        short: f[1].into(),
        author: crate::conflicts::masked(store, f[2]),
        date: f[3].into(),
        subject: crate::conflicts::masked(store, f[4]),
        merge: f[5].split_whitespace().count() > 1,
        agent: (!agent.is_empty()).then(|| crate::conflicts::masked(store, agent)),
    })
}

/// The commit a revert would undo and the repository files it would change.
/// Nothing is written.
pub fn plan_revert(store: &Store, commit: &str) -> Result<(Commit, Vec<String>)> {
    let hash = resolve(store, commit)?;
    let info = commit_info(store, &hash)?;
    let parent = if info.merge { "^1" } else { "^" };
    let out = store.git(&["diff", "--name-only", &format!("{hash}{parent}"), &hash])?;
    let files = out
        .stdout
        .lines()
        .map(|f| crate::conflicts::masked(store, f))
        .collect();
    Ok((info, files))
}

/// Create a revert commit for `commit`. A conflicting revert is abandoned and
/// leaves the repository as it was.
pub fn revert(store: &Store, commit: &str, agent: Option<&str>) -> Result<Commit> {
    let hash = resolve(store, commit)?;
    let _lock = store.lock()?;
    crate::track::ensure_repo_clean(store)?;
    if store.git(&["diff", "--cached", "--quiet"]).is_err() {
        bail!("commit or unstage the staged changes before reverting");
    }
    let parents = store
        .git(&["rev-list", "--parents", "-n1", &hash])?
        .stdout
        .split_whitespace()
        .count()
        - 1;
    let mut args = vec!["revert", "--no-edit"];
    if parents > 1 {
        args.extend(["-m", "1"]);
    }
    args.push(&hash);
    if store.git(&args).is_err() {
        if store.repo().join(".git/REVERT_HEAD").exists() {
            store
                .git(&["revert", "--abort"])
                .context("aborting the conflicting revert")?;
        }
        return Err(DwError::new(
            Kind::Conflict,
            format!("reverting {commit} failed; no revert commit was created"),
        )
        .into());
    }
    if let Some(a) = agent {
        store
            .git(&[
                "commit",
                "--amend",
                "--quiet",
                "--no-edit",
                "--trailer",
                &format!("Agent: {a}"),
            ])
            .context("adding the agent trailer")?;
    }
    commit_info(store, "HEAD")
}
