use crate::{config::Config, process};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitFile {
    pub path: String,
    pub original: Option<String>,
    pub index: char,
    pub worktree: char,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitState {
    pub branch: String,
    pub head: String,
    pub upstream: String,
    pub ahead: usize,
    pub behind: usize,
    pub files: Vec<GitFile>,
    pub commits: Vec<String>,
    pub error: Option<String>,
}
pub fn run(c: &Config, args: &[&str]) -> Result<String> {
    process::run(&c.git, &process::strings(args), Some(&c.source), None)
}
pub fn args(c: &Config, args: Vec<String>) -> Result<String> {
    process::run(&c.git, &args, Some(&c.source), None)
}
pub fn status(c: &Config) -> Result<GitState> {
    let mut state = GitState {
        branch: run(c, &["symbolic-ref", "--short", "HEAD"])
            .unwrap_or_else(|_| "detached".into())
            .trim()
            .into(),
        head: run(c, &["rev-parse", "HEAD"])
            .unwrap_or_default()
            .trim()
            .into(),
        upstream: run(c, &["rev-parse", "--abbrev-ref", "@{upstream}"])
            .unwrap_or_default()
            .trim()
            .into(),
        ..Default::default()
    };
    state.files = parse_status(&run(
        c,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?);
    if !state.upstream.is_empty() {
        let counts = run(
            c,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )?;
        let vals: Vec<_> = counts.split_whitespace().collect();
        state.ahead = vals.first().and_then(|v| v.parse().ok()).unwrap_or(0);
        state.behind = vals.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    state.commits = run(c, &["log", "-20", "--format=%h %s"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    Ok(state)
}
pub fn parse_status(text: &str) -> Vec<GitFile> {
    let mut parts = text.split('\0');
    let mut files = vec![];
    while let Some(part) = parts.next() {
        if part.len() < 3 || !part.is_char_boundary(3) {
            continue;
        }
        let index = part.as_bytes()[0] as char;
        let worktree = part.as_bytes()[1] as char;
        let original = if index == 'R' || index == 'C' || worktree == 'R' || worktree == 'C' {
            parts.next().map(str::to_string)
        } else {
            None
        };
        files.push(GitFile {
            path: part[3..].into(),
            index,
            worktree,
            original,
        });
    }
    files
}
pub fn diff(c: &Config, paths: &[String], cached: bool) -> Result<String> {
    let mut a = process::strings(&["--literal-pathspecs", "diff", "--no-ext-diff", "--no-color"]);
    if cached {
        a.push("--cached".into());
    }
    a.push("--".into());
    a.extend_from_slice(paths);
    let mut text = args(c, a)?;
    if !cached {
        let state = status(c)?;
        for file in state
            .files
            .iter()
            .filter(|f| f.index == '?' && (paths.is_empty() || paths.contains(&f.path)))
        {
            let absolute = c.source.join(&file.path);
            let output = process::output(
                &c.git,
                &process::strings(&[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-color",
                    "--",
                    "/dev/null",
                    &absolute.to_string_lossy(),
                ]),
                Some(&c.source),
                None,
            )?;
            if output.status.code().is_some_and(|code| code <= 1) {
                text.push_str(&String::from_utf8_lossy(&output.stdout));
            }
        }
    }
    Ok(text)
}

pub fn push_target(c: &Config) -> Result<(String, String, String)> {
    let branch = run(c, &["symbolic-ref", "--short", "HEAD"])?;
    let branch = branch.trim().to_string();
    let remote = run(c, &["config", "--get", &format!("branch.{branch}.remote")])?;
    let remote = remote.trim().to_string();
    let target = run(c, &["config", "--get", &format!("branch.{branch}.merge")])?;
    let target = target.trim().to_string();
    if remote.starts_with('-') || !target.starts_with("refs/heads/") {
        anyhow::bail!("Invalid branch upstream");
    }
    Ok((remote, branch, target))
}

pub fn index_path(c: &Config) -> Result<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(
        run(
            c,
            &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        )?
        .trim(),
    ))
}
