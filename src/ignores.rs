//! One reviewed action for chezmoi's target paths and Git's source paths.
use crate::{
    backend,
    config::{Config, atomic_write, clean_relative},
    git, process,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const DEFAULT_MARKER: &str = "# dot: developer ignore defaults v1";
const DEFAULT_GROUPS: &[(&str, &[&str])] = &[
    (
        "macOS and Linux desktop clutter",
        &[
            ".DS_Store",
            "._*",
            ".AppleDouble",
            ".LSOverride",
            ".Spotlight-V100",
            ".Trashes",
            ".fseventsd",
            ".TemporaryItems",
            ".VolumeIcon.icns",
            ".directory",
            ".Trash-*",
            ".nfs*",
            "Thumbs.db",
            "Desktop.ini",
        ],
    ),
    (
        "JavaScript and frontend dependencies / output",
        &[
            "node_modules",
            ".npm",
            ".pnpm-store",
            ".yarn/cache",
            ".yarn/unplugged",
            ".next",
            ".nuxt",
            ".output",
            ".svelte-kit",
            ".turbo",
            ".parcel-cache",
            ".vite",
            ".angular/cache",
        ],
    ),
    (
        "Python environments and caches",
        &[
            "__pycache__",
            "*.pyc",
            "*.pyo",
            ".venv",
            "venv",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".tox",
            ".nox",
            ".hypothesis",
            "*.egg-info",
            ".coverage",
            ".coverage.*",
            "htmlcov",
        ],
    ),
    (
        "Rust, JVM, Swift, Dart and native build output",
        &[
            "target",
            ".gradle",
            ".build",
            "DerivedData",
            ".dart_tool",
            ".pub-cache",
            "CMakeFiles",
            "CMakeCache.txt",
            "cmake-build-*",
            "*.o",
            "*.obj",
            "*.class",
        ],
    ),
    (
        "Build output, test coverage and tool caches",
        &[
            "build",
            "dist",
            "coverage",
            ".nyc_output",
            ".cache",
            ".sass-cache",
            ".bundle/cache",
            ".terraform",
            "*.tfstate",
            "*.tfstate.*",
        ],
    ),
    (
        "Editor temporary files",
        &[
            "*.swp",
            "*.swo",
            "*~",
            ".#*",
            "npm-debug.log*",
            "yarn-debug.log*",
            "yarn-error.log*",
            "pnpm-debug.log*",
        ],
    ),
];

// Rooted rules target HOME paths, not similarly named project source folders.
const MACHINE_GROUPS: &[(&str, &[&str])] = &[
    (
        "Trash, histories and operating-system data",
        &[
            "/.Trash",
            "/.trash",
            "/.local/share/Trash",
            "/.local/state",
            "/.bash_history",
            "/.zsh_history*",
            "/.zsh_sessions",
            "/.zcompcache",
            "/.zcompdump*",
            "/.python_history",
            "/.node_repl_history",
            "/.sqlite_history",
            "/.lesshst",
            "/.viminfo",
            "/.wget-hsts",
            "/Library",
            "/Applications",
            "/Desktop",
            "/Documents",
            "/Downloads",
            "/Movies",
            "/Music",
            "/Pictures",
            "/Videos",
            "/Public",
        ],
    ),
    (
        "Personal data and project workspaces",
        &[
            "/projects",
            "/Projects",
            "/work",
            "/workspace",
            "/workspaces",
            "/repos",
            "/src",
            "/notes",
            "/journal",
            "/learn",
            "/iCloud Drive (Archive)",
            "/qmk_firmware",
            "/chrome-headless-shell",
            "/dwhelper",
            "/go/pkg",
        ],
    ),
    (
        "Installed toolchains, packages and local models",
        &[
            "/.android",
            "/.bob",
            "/.bundle",
            "/.cargo",
            "/.rustup",
            "/.bun",
            "/.nvm",
            "/.gem",
            "/.cocoapods",
            "/.expo",
            "/.ollama",
            "/.pnpm-state",
            "/.proto",
            "/.docker",
            "/.swiftpm/security",
            "/.swiftpm/cache",
            "/.local/share/bob",
            "/.local/share/uv",
            "/.local/share/nvim",
            "/.local/share/atuin",
            "/.local/share/opencode",
            "/.local/share/opentui",
            "/.local/share/dot-cli",
            "/.local/share/fish/fish_history",
            "/.vscode/extensions",
            "/.vscode/cli",
            "/.vscode-oss/extensions",
            "/.vscode-oss/cli",
            "/.cursor/extensions",
            "/.cursor/cli",
            "/.vscode-server",
            "/.cursor-server",
            ".venv-*",
            ".fastembed_cache",
            "*.log",
            "*.pid",
            "*.sock",
            "*.socket",
        ],
    ),
    (
        "Agent runtime state (configs, hooks and skills remain available)",
        &[
            "/.codex/sessions",
            "/.codex/archived_sessions",
            "/.codex/log",
            "/.codex/tmp",
            "/.codex/shell_snapshots",
            "/.codex/history.jsonl",
            "/.codex/state_*.sqlite*",
            "/.codex/logs_*.sqlite*",
            "/.codex/models_cache.json",
            "/.codex/.codex-global-state.json",
            "/.codex/plugins/cache",
            "/.codex/plugins/marketplaces",
            "/.claude/projects",
            "/.claude/debug",
            "/.claude/cache",
            "/.claude/session-env",
            "/.claude/shell-snapshots",
            "/.claude/file-history",
            "/.claude/paste-cache",
            "/.claude/todos",
            "/.claude/telemetry",
            "/.claude/statsig",
            "/.claude/history.jsonl",
            "/.claude/stats-cache.json",
            "/.gemini/tmp",
            "/.gemini/history",
            "/.copilot/session-state",
            "/.copilot/logs",
            "/.config/zed/conversations",
            "/.config/zed/embeddings",
        ],
    ),
    (
        "Local authentication and identity files",
        &[
            "/.mcp-auth",
            "/.app-store/auth",
            "/.codex/auth.json",
            "/.claude/.credentials.json",
            "/.claude/credentials.json",
            "/.gemini/oauth_creds.json",
            "/.gemini/google_accounts.json",
            "/.gemini/mcp-oauth-tokens.json",
            "/.config/gh/hosts.yml",
            "/.aws/credentials",
            "/.aws/sso/cache",
            "/.aws/cli/cache",
            "/.ssh/id_*",
            "/.ssh/known_hosts*",
            "/.ssh/authorized_keys",
            "/.gnupg",
            "/.netrc",
            "/.local/share/keyrings",
            "/.config/gcloud/credentials.db",
            "/.config/gcloud/access_tokens.db",
            "/.config/gcloud/application_default_credentials.json",
        ],
    ),
];
const MACHINE_MARKER: &str = "# dot: machine ignore defaults v2";

// Native evaluation preserves nested rules, machine templates and ! exceptions.
// Directories are pruned by chezmoi before walking their contents.
pub fn eligible(c: &Config, candidates: &[String]) -> Result<BTreeSet<String>> {
    let mut eligible = BTreeSet::new();
    let mut pending: BTreeSet<String> = candidates.iter().cloned().collect();
    while !pending.is_empty() {
        let chunk: Vec<_> = pending.iter().take(128).cloned().collect();
        let mut args = process::strings(&[
            "unmanaged",
            "--path-style=relative",
            "-0",
            "--include=files,symlinks,dirs",
            "--",
        ]);
        for path in &chunk {
            pending.remove(path);
            args.push(c.target(path)?.to_string_lossy().into_owned());
        }
        for path in backend::chez_args(c, args)?
            .split('\0')
            .filter(|s| !s.is_empty())
        {
            let full = c.target(path)?;
            if full.starts_with(&c.source)
                || full.starts_with(&c.state)
                || Path::new(path)
                    .components()
                    .any(|p| p.as_os_str() == ".git")
            {
                continue;
            }
            // unmanaged emits the highest unmanaged directory, not every leaf.
            // Expand only directories it accepted, so ignored trees are never walked.
            if fs::symlink_metadata(&full)?.is_dir() {
                for entry in fs::read_dir(full)? {
                    let entry = entry?;
                    let child = c.relative(&entry.path().to_string_lossy())?;
                    if entry.file_name() != ".git"
                        && !entry.path().starts_with(&c.source)
                        && !entry.path().starts_with(&c.state)
                    {
                        pending.insert(child);
                    }
                }
            } else {
                eligible.insert(path.to_owned());
            }
            if eligible.len() > 10000 {
                bail!("Select a smaller folder (more than 10000 eligible files)");
            }
        }
    }
    Ok(eligible)
}

#[derive(Serialize, Deserialize)]
pub struct Edit {
    path: PathBuf,
    before: Option<String>,
    content: String,
}
#[derive(Serialize, Deserialize)]
pub struct Change {
    pub edits: Vec<Edit>,
    pub untrack: Vec<String>,
    targets: Vec<String>,
    sources: Vec<String>,
    pub preview: String,
}

fn safe_file(c: &Config, path: &Path) -> Result<()> {
    let rel = path
        .strip_prefix(&c.source)
        .context("Ignore file must be inside the settings repository")?;
    clean_relative(rel)?;
    let mut full = c.source.clone();
    for part in rel.components() {
        full.push(part);
        if fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Ignore file or parent is a symlink: {}", full.display());
        }
    }
    Ok(())
}
fn read(c: &Config, path: PathBuf) -> Result<Edit> {
    safe_file(c, &path)?;
    let before = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    Ok(Edit {
        path,
        content: before.clone().unwrap_or_default(),
        before,
    })
}
fn files(c: &Config) -> Result<(PathBuf, Vec<Edit>)> {
    let root = PathBuf::from(backend::chez(c, &["source-path"])?.trim());
    let mut chez = root.join(".chezmoiignore");
    if root.join(".chezmoiignore.tmpl").exists() {
        if chez.exists() {
            bail!(
                "Both .chezmoiignore and .chezmoiignore.tmpl exist; consolidate them before adding rules"
            );
        }
        chez = root.join(".chezmoiignore.tmpl");
    }
    Ok((
        root,
        vec![read(c, chez)?, read(c, c.source.join(".gitignore"))?],
    ))
}
fn append(edit: &mut Edit, rules: impl IntoIterator<Item = String>) -> String {
    let mut diff = String::new();
    for rule in rules {
        if !edit.content.lines().any(|line| line == rule) {
            if !edit.content.is_empty() && !edit.content.ends_with('\n') {
                edit.content.push('\n');
            }
            edit.content.push_str(&rule);
            edit.content.push('\n');
            diff.push_str(&format!("+{rule}\n"));
        }
    }
    diff
}
fn literal(path: &str) -> Result<String> {
    clean_relative(Path::new(path))?;
    if path.chars().any(char::is_control)
        || path.contains("{{")
        || path.contains('#')
        || path.trim() != path
    {
        bail!("This filename needs a manually escaped ignore rule: {path:?}");
    }
    if Path::new(path)
        .components()
        .any(|p| p.as_os_str() == ".git")
    {
        bail!("Git internals cannot be ignored through this action");
    }
    let mut escaped = String::new();
    for ch in path.chars() {
        if "\\*?[]{}! ".contains(ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    Ok(escaped)
}
fn source_relative(c: &Config, path: &Path) -> Result<String> {
    clean_relative(
        path.strip_prefix(&c.source)
            .context("Saved copy is outside the repository")?,
    )
}

pub fn home_paths(c: &Config, paths: &[String], from_git: bool) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|p| {
            if from_git {
                clean_relative(Path::new(p))?;
                let source = c.source.join(p);
                safe_file(c, &source)?;
                let target = backend::chez(c, &["target-path", "--", &source.to_string_lossy()])?;
                c.relative(target.trim())
            } else {
                c.relative(p)
            }
        })
        .collect()
}

pub fn prepare(c: &Config, paths: &[String], git_paths: Option<&[String]>) -> Result<Change> {
    if paths.is_empty() {
        bail!("Select a file or folder to ignore");
    }
    let (root, mut edits) = files(c)?;
    let mut preview =
        "Ignore in chezmoi and Git. Original files and saved copies stay on disk.\n".to_string();
    let mut sources = BTreeSet::new();
    let mut chez_rules = vec![];
    let mut git_rules = vec![];
    let mut lookup = vec![];
    for (i, path) in paths.iter().enumerate() {
        let rule = literal(path)?;
        let target = c.target(path)?;
        let source = if let Some(selected) = git_paths {
            c.source.join(&selected[i])
        } else if let Ok(source) =
            backend::chez(c, &["source-path", "--", &target.to_string_lossy()])
        {
            PathBuf::from(source.trim())
        } else {
            lookup.push(path.clone());
            // No active saved copy exists. Use the conventional source spelling; chezmoi
            // will prevent capture, so no attributed source file will be created.
            root.join(
                path.split('/')
                    .map(|p| {
                        p.strip_prefix('.')
                            .map(|p| format!("dot_{p}"))
                            .unwrap_or_else(|| p.into())
                    })
                    .collect::<Vec<_>>()
                    .join("/"),
            )
        };
        let source = source_relative(c, &source)?;
        if [
            ".gitignore",
            ".chezmoiignore",
            ".chezmoiignore.tmpl",
            ".chezmoiroot",
        ]
        .contains(
            &Path::new(&source)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
        ) {
            bail!("Ignore control files cannot ignore themselves");
        }
        chez_rules.push(rule.clone());
        if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir())
            || c.source.join(&source).is_dir()
        {
            chez_rules.push(format!("{rule}/**"));
        }
        git_rules.push(format!("/{}", literal(&source)?));
        // Also cover ordinary files created directly inside the repository.
        let raw = source_relative(c, &root.join(path))?;
        git_rules.push(format!("/{}", literal(&raw)?));
        sources.insert(source);
        sources.insert(raw);
    }
    let tracked = git::run(c, &["ls-files", "-z"])?;
    // Defaults may already hide a saved copy from `source-path`. Resolve actual
    // indexed files in reverse before stopping tracking; attributed names cannot
    // be guessed from HOME names (e.g. private_dot_config/private_x.tmpl).
    if !lookup.is_empty() {
        for source in tracked.split('\0').filter(|p| !p.is_empty()) {
            if !c.source.join(source).exists() {
                continue;
            }
            let name = Path::new(source)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            if name == ".gitignore"
                || Path::new(source)
                    .components()
                    .any(|p| p.as_os_str().to_string_lossy().starts_with(".chezmoi"))
            {
                continue;
            }
            let possible = lookup.iter().any(|target| {
                if c.home.join(target).is_dir() {
                    return true;
                }
                let name = name.strip_suffix(".tmpl").unwrap_or(&name);
                let base = Path::new(target)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let encoded = base
                    .strip_prefix('.')
                    .map(|b| format!("dot_{b}"))
                    .unwrap_or_else(|| base.to_string());
                name == base || name == encoded || name.ends_with(&format!("_{encoded}"))
            });
            if !possible {
                continue;
            }
            let target = backend::chez(
                c,
                &[
                    "target-path",
                    "--",
                    &c.source.join(source).to_string_lossy(),
                ],
            )?;
            let target = c.relative(target.trim())?;
            if lookup
                .iter()
                .any(|p| target == *p || target.starts_with(&format!("{p}/")))
            {
                git_rules.push(format!("/{}", literal(source)?));
                sources.insert(source.to_string());
            }
        }
    }
    for (edit, rules) in edits.iter_mut().zip([chez_rules, git_rules]) {
        let diff = append(edit, rules);
        preview.push_str(&format!("\n{}\n{diff}", edit.path.display()));
    }
    let untrack: Vec<_> = tracked
        .split('\0')
        .filter(|p| {
            !p.is_empty()
                && sources
                    .iter()
                    .any(|s| *p == s || p.starts_with(&format!("{s}/")))
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !untrack.is_empty() {
        untrack_files(c, &untrack, true).context("Git has staged changes that cannot be safely ignored. Commit or unstage those files first")?;
        preview.push_str(&format!(
            "\nStop Git tracking (stage removal; keep copies on disk):\n{}\n",
            untrack.join("\n")
        ));
    }
    if edits
        .iter()
        .all(|e| e.before.as_deref().unwrap_or_default() == e.content)
        && untrack.is_empty()
    {
        bail!("These ignore rules already exist");
    }
    preview.push_str("\nExisting ! exceptions still take priority in chezmoi. To undo, use Recovery before making more changes, or edit both ignore files and re-add the saved copies to Git.");
    Ok(Change {
        edits,
        untrack,
        targets: paths.to_vec(),
        sources: sources.into_iter().collect(),
        preview,
    })
}

pub fn defaults(c: &Config) -> Result<Change> {
    let (_, mut edits) = files(c)?;
    let mut preview = "Add developer defaults to both ignore files. Existing custom rules stay in place.\nFiles already tracked by Git stay tracked; select them and press i to stop tracking.\n".to_string();
    let root = PathBuf::from(backend::chez(c, &["source-path"])?.trim());
    let source_prefix = root.strip_prefix(&c.source)?.to_string_lossy();
    for (marker, groups) in [
        (DEFAULT_MARKER, DEFAULT_GROUPS),
        (MACHINE_MARKER, MACHINE_GROUPS),
    ] {
        for (i, edit) in edits.iter_mut().enumerate() {
            // Versioned additions preserve edits to previously installed defaults.
            if edit.content.lines().any(|l| l == marker) {
                continue;
            }
            let mut block = format!(
                "\n{marker}\n# Editable defaults. Remove or narrow rules you want to keep.\n"
            );
            for (title, patterns) in groups {
                block.push_str(&format!("\n# {title}\n"));
                for pattern in *patterns {
                    let rooted = pattern.starts_with('/');
                    let pattern = pattern.trim_start_matches('/');
                    let chez_pattern = if rooted {
                        pattern.to_owned()
                    } else {
                        format!("**/{pattern}")
                    };
                    if i == 0 {
                        block.push_str(&format!("{chez_pattern}\n{chez_pattern}/**\n"));
                    } else {
                        let prefix = if rooted {
                            if source_prefix.is_empty() {
                                "/".into()
                            } else {
                                format!("/{source_prefix}/")
                            }
                        } else {
                            "**/".to_string()
                        };
                        let mut variants = BTreeSet::from([pattern.to_owned()]);
                        let encoded: Vec<_> = pattern
                            .split('/')
                            .map(|p| {
                                p.strip_prefix('.')
                                    .map(|p| format!("dot_{p}"))
                                    .unwrap_or_else(|| p.into())
                            })
                            .collect();
                        variants.insert(encoded.join("/"));
                        // Ordinary and private source paths are both common. Include
                        // each directory attribute without wildcards matching neighbours.
                        for attr in ["private_", "exact_", "private_exact_", "symlink_"] {
                            if !encoded.last().is_some_and(|s| s.starts_with('*')) {
                                let mut parts = encoded.clone();
                                if let Some(last) = parts.last_mut() {
                                    *last = format!("{attr}{last}");
                                }
                                variants.insert(parts.join("/"));
                            }
                        }
                        if encoded.len() > 1 {
                            let private: Vec<_> =
                                encoded.iter().map(|p| format!("private_{p}")).collect();
                            variants.insert(private.join("/"));
                            let mut parts = encoded.clone();
                            parts[0] = format!("private_{}", parts[0]);
                            variants.insert(parts.join("/"));
                        }
                        for path in variants {
                            block.push_str(&format!("{prefix}{path}\n"));
                        }
                    }
                }
            }
            block.push_str("# dot: end ignore defaults\n");
            // Protect existing template whitespace trimming and Git exception order.
            edit.content = if i == 0 && !edit.content.is_empty() {
                format!("{block}\n{{{{ \"\\n\" }}}}{}", edit.content)
            } else {
                format!("{block}\n{}", edit.content)
            };
            preview.push_str(&format!(
                "\n{}\n{}",
                edit.path.display(),
                block.lines().map(|l| format!("+{l}\n")).collect::<String>()
            ));
        }
    }
    Ok(Change {
        edits,
        untrack: vec![],
        targets: vec![],
        sources: vec![],
        preview,
    })
}

fn write(edit: &Edit, content: Option<&str>) -> Result<()> {
    let mode = fs::metadata(&edit.path).ok().map(|m| m.permissions());
    if let Some(content) = content {
        atomic_write(&edit.path, content.as_bytes())?;
        if let Some(mode) = mode {
            fs::set_permissions(&edit.path, mode)?;
        }
    } else if edit.path.exists() {
        fs::remove_file(&edit.path)?;
    }
    Ok(())
}
fn untrack_files(c: &Config, paths: &[String], dry_run: bool) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = process::strings(&["--literal-pathspecs", "rm", "--cached"]);
    if dry_run {
        args.push("--dry-run".into());
    }
    args.push("--".into());
    args.extend_from_slice(paths);
    git::args(c, args)?;
    Ok(())
}
pub fn apply(c: &Config, payload: &str) -> Result<String> {
    let change: Change = serde_json::from_str(payload)?;
    for edit in &change.edits {
        safe_file(c, &edit.path)?;
    }
    let result = (|| -> Result<()> {
        for edit in &change.edits {
            write(edit, Some(&edit.content))?;
        }
        if !change.targets.is_empty() {
            let managed = backend::snapshot(c)?.managed;
            let existing: Vec<_> = change
                .targets
                .iter()
                .filter(|p| fs::symlink_metadata(c.home.join(p)).is_ok())
                .cloned()
                .collect();
            let kept = eligible(c, &existing)?;
            if !kept.is_empty()
                || change.targets.iter().any(|p| {
                    managed
                        .keys()
                        .any(|m| m == p || m.starts_with(&format!("{p}/")))
                })
            {
                bail!(
                    "An existing chezmoi exception keeps a selected path included. Edit the ! rule in .chezmoiignore first; both ignore files have been restored"
                );
            }
            let mut input = change.sources.join("\0").into_bytes();
            input.push(0);
            let output = process::run(
                &c.git,
                &process::strings(&["check-ignore", "--no-index", "--stdin", "-z"]),
                Some(&c.source),
                Some(&input),
            )?;
            let ignored: BTreeSet<_> = output.split('\0').collect();
            if change.sources.iter().any(|p| !ignored.contains(p.as_str())) {
                bail!(
                    "An existing Git exception keeps a selected path included. Update that rule first; both ignore files have been restored"
                );
            }
        }
        untrack_files(c, &change.untrack, false)
    })();
    if let Err(error) = result {
        for edit in change.edits.iter().rev() {
            write(edit, edit.before.as_deref()).context("Ignore update failed; restoring the original rules also failed. Use the operation backup")?;
        }
        return Err(error);
    }
    Ok(format!(
        "Updated both ignore files. {} saved copies removed from Git tracking; all files remain on disk. Review the changes in Git before committing.",
        change.untrack.len()
    ))
}

pub fn install_defaults(c: &Config) -> Result<()> {
    let _lock = crate::operations::Lock::acquire(c)?;
    apply(c, &serde_json::to_string(&defaults(c)?)?)?;
    Ok(())
}
