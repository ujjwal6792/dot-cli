use crate::{
    backend,
    config::{Config, atomic_write, clean_relative},
    git, process,
};
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Capture,
    Apply,
    Restore,
    Forget,
    Trash,
    Ignore,
    IgnoreDefaults,
    Enable,
    Disable,
    Stage,
    Unstage,
    Discard,
    Commit,
    Fetch,
    Pull,
    Push,
    EditSource,
    Setting,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Capture => "Capture HOME → source",
            Self::Apply => "Apply source → HOME",
            Self::Restore => "Restore source → HOME",
            Self::Ignore => "Ignore in chezmoi and Git",
            Self::IgnoreDefaults => "Add developer ignore defaults",
            Self::Forget => "Stop tracking (shared source)",
            Self::Trash => "Move untracked originals to Trash",
            Self::Enable => "Enable automatic updates here",
            Self::Disable => "Disable automatic updates here",
            Self::Stage => "Stage source changes",
            Self::Unstage => "Unstage source changes",
            Self::Discard => "Discard source changes → HEAD (index preserved)",
            Self::Commit => "Commit actual staged changes",
            Self::Fetch => "Fetch remote",
            Self::Pull => "Pull fast-forward",
            Self::Push => "Push configuration commits",
            Self::Setting => "Change shared/OS/machine setting",
            Self::EditSource => "Edit source",
        }
    }
    pub fn files(self) -> bool {
        matches!(
            self,
            Self::Capture
                | Self::Apply
                | Self::Restore
                | Self::Forget
                | Self::Trash
                | Self::Enable
                | Self::Disable
                | Self::EditSource
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub action: Action,
    pub paths: Vec<String>,
    pub payload: Option<String>,
    pub fingerprint: String,
    pub preview: String,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub action: Action,
    pub succeeded: Vec<String>,
    pub failed: Option<String>,
    pub not_attempted: Vec<String>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub time: chrono::DateTime<chrono::Utc>,
    pub id: String,
    pub actor: String,
    pub intent: String,
    pub paths: Vec<String>,
    pub outcome: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedFile {
    relative: String,
    kind: String,
    mode: u32,
    link: Option<PathBuf>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Recovery {
    plan: Plan,
    source: Vec<SavedFile>,
    home: Vec<SavedFile>,
    #[serde(default)]
    index: Option<SavedFile>,
    after: String,
    result: Outcome,
}

struct LockedFile(File);
impl Drop for LockedFile {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
pub struct Lock {
    _local: LockedFile,
    _source: Option<LockedFile>,
}
impl Lock {
    pub fn acquire(c: &Config) -> Result<Self> {
        fs::create_dir_all(&c.state)?;
        fs::set_permissions(&c.state, fs::Permissions::from_mode(0o700))?;
        let local = lock_file(&c.state.join("operation.lock"))?;
        let source = git::run(
            c,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .map(|p| lock_file(&PathBuf::from(p.trim()).join("dot-operation.lock")))
        .transpose()?;
        Ok(Self {
            _local: local,
            _source: source,
        })
    }
}
fn lock_file(path: &Path) -> Result<LockedFile> {
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    f.try_lock_exclusive().with_context(|| {
        format!(
            "Another dot operation is running; lock {} is busy",
            path.display()
        )
    })?;
    Ok(LockedFile(f))
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn add_file(h: &mut Sha256, path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            h.update(m.permissions().mode().to_le_bytes());
            if m.file_type().is_symlink() {
                h.update(b"link");
                h.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
            } else if m.is_file() {
                h.update(b"file");
                h.update(fs::read(path)?);
            } else {
                h.update(b"directory");
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => h.update(b"missing"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
pub fn fingerprint(c: &Config, paths: &[String], home: bool) -> Result<String> {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(c)?);
    for e in walkdir::WalkDir::new(&c.source)
        .sort_by_file_name()
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let e = e?;
        h.update(
            e.path()
                .strip_prefix(&c.source)?
                .as_os_str()
                .as_encoded_bytes(),
        );
        add_file(&mut h, e.path())?;
    }
    if home {
        for p in paths {
            h.update(p);
            add_file(&mut h, &c.target(p)?)?;
        }
    }
    for p in crate::config::native_config_paths(c) {
        add_file(&mut h, &p)?;
        if p.is_file() {
            h.update(fs::read(p)?);
        }
    }
    if let Some(p) = &c.runtime_config {
        add_file(&mut h, p)?;
    }
    for a in [
        vec!["rev-parse", "HEAD"],
        vec!["rev-parse", "@{upstream}"],
        vec!["symbolic-ref", "HEAD"],
    ] {
        h.update(git::run(c, &a).unwrap_or_default());
    }
    if let Ok(p) = git::run(
        c,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ) {
        add_file(&mut h, Path::new(p.trim()))?;
    }
    h.update(git::run(c, &["config", "--null", "--list", "--show-origin"]).unwrap_or_default());
    Ok(format!("{:x}", h.finalize()))
}
fn plan_id(p: &Plan) -> Result<String> {
    Ok(hash(&serde_json::to_vec(&(
        p.action,
        &p.paths,
        &p.payload,
        &p.fingerprint,
        &p.preview,
        &p.created_at,
    ))?)[..24]
        .into())
}
pub fn plan(
    c: &Config,
    action: Action,
    paths: &[String],
    mut payload: Option<String>,
) -> Result<Plan> {
    let _lock = Lock::acquire(c)?;
    let snap = if action.files() {
        Some(backend::snapshot(c)?)
    } else {
        None
    };
    let from_git = action == Action::Ignore && payload.as_deref() == Some("source");
    let targets = if action == Action::Trash {
        let (targets, digest) = crate::trash_originals::prepare(c, snap.as_ref().unwrap(), paths)?;
        payload = Some(digest);
        targets
    } else if action == Action::Ignore {
        crate::ignores::home_paths(c, paths, from_git)?
    } else if let Some(s) = &snap {
        backend::exact_targets(c, s, paths, action == Action::Capture)?
    } else {
        for p in paths {
            clean_relative(Path::new(p))?;
        }
        paths.to_vec()
    };
    if matches!(action, Action::Stage | Action::Unstage | Action::Discard) && targets.is_empty() {
        bail!("Select source files first");
    }
    if action == Action::Capture {
        for p in &targets {
            if let Some(m) = snap.as_ref().and_then(|s| s.managed.get(p))
                && (m.source_relative.ends_with(".tmpl")
                    || Path::new(&m.source_relative)
                        .components()
                        .any(|p| p.as_os_str().to_string_lossy().starts_with("modify_")))
            {
                bail!(
                    "{p} is templated or modified: edit its source or mapped setting; capture would destroy its behavior"
                );
            }
        }
    }
    let preview = match action {
        Action::Trash => crate::trash_originals::preview(c, &targets)?,
        Action::Apply | Action::Restore => backend::preview(c, &targets, false)?,
        Action::Capture => {
            let managed: Vec<_> = targets
                .iter()
                .filter(|p| snap.as_ref().unwrap().managed.contains_key(*p))
                .cloned()
                .collect();
            let mut text = backend::preview(c, &managed, true)?;
            for p in targets.iter().filter(|p| !managed.contains(p)) {
                text.push_str(&format!("\nNEW source from HOME: {p}\n"));
                if let Ok(t) = fs::read_to_string(c.target(p)?) {
                    text.push_str(&t);
                }
            }
            text
        }
        Action::Ignore | Action::IgnoreDefaults => {
            let change = if action == Action::IgnoreDefaults {
                crate::ignores::defaults(c)?
            } else {
                crate::ignores::prepare(c, &targets, from_git.then_some(paths))?
            };
            let preview = change.preview.clone();
            payload = Some(serde_json::to_string(&change)?);
            preview
        }
        Action::Forget => format!(
            "Remove these configurations from SHARED source; keep HOME files. Publishing changes what other machines can manage.\n{}",
            targets.join("\n")
        ),
        Action::Enable | Action::Disable => format!(
            "Change only automatic update scope on {}. No configuration file writes.\n{}",
            c.machine,
            targets.join("\n")
        ),
        Action::Stage => git::diff(c, &targets, false)?,
        Action::Discard => {
            let mut a = process::strings(&[
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-color",
                "HEAD",
                "--",
            ]);
            a.extend(targets.clone());
            git::args(c, a)?
        }
        Action::Unstage => git::diff(c, &targets, true)?,
        Action::Commit => {
            if payload.as_ref().is_none_or(|s| s.trim().is_empty()) {
                bail!("Commit message is required");
            }
            let d = git::diff(c, &[], true)?;
            if d.is_empty() {
                bail!("No staged changes");
            }
            format!("All ACTUAL staged changes will be committed:\n{d}")
        }
        Action::Fetch => "Fetch remote refs. Configuration files and HOME stay unchanged.".into(),
        Action::Pull => {
            let g = git::status(c)?;
            if !g.files.is_empty() {
                bail!("Source has local changes; resolve them before pull");
            }
            if g.upstream.is_empty() {
                bail!("No upstream configured");
            }
            format!(
                "Fetch and fast-forward {} from {}. HOME application is separate.",
                g.branch, g.upstream
            )
        }
        Action::Push => {
            let g = git::status(c)?;
            if g.upstream.is_empty() {
                bail!("No upstream: set one with Git before pushing");
            }
            format!(
                "Push committed configuration history on {}. Uncommitted edits will not be published.\n{}",
                g.branch,
                git::run(c, &["log", "--oneline", "@{upstream}..HEAD"])?
            )
        }
        Action::Setting => {
            let change: crate::settings::Change =
                serde_json::from_str(payload.as_deref().context("Setting change missing")?)?;
            crate::settings::update(c, &change, false)?
        }
        Action::EditSource => {
            if targets.len() != 1 || payload.is_none() {
                bail!("Source editing requires one file and new content");
            }
            format!(
                "Replace source for {} with:\n{}",
                targets[0],
                payload.as_deref().unwrap()
            )
        }
    };
    let mut p = Plan {
        id: String::new(),
        action,
        paths: targets,
        payload,
        fingerprint: String::new(),
        preview,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    p.fingerprint = fingerprint(c, &p.paths, action.files())?;
    p.id = plan_id(&p)?;
    atomic_write(
        &c.state.join("plans").join(format!("{}.json", p.id)),
        &serde_json::to_vec_pretty(&p)?,
    )?;
    Ok(p)
}
fn saved(root: &Path, relative: &str, backup: &Path) -> Result<SavedFile> {
    let p = root.join(relative);
    let mut out = SavedFile {
        relative: relative.into(),
        kind: "missing".into(),
        mode: 0,
        link: None,
    };
    if let Ok(m) = fs::symlink_metadata(&p) {
        out.mode = m.permissions().mode();
        if m.file_type().is_symlink() {
            out.kind = "link".into();
            out.link = Some(fs::read_link(&p)?);
        } else if m.is_file() {
            out.kind = "file".into();
            let dst = backup.join(relative);
            fs::create_dir_all(dst.parent().unwrap())?;
            fs::copy(&p, dst)?;
        } else {
            out.kind = "dir".into();
        }
    }
    Ok(out)
}
fn backup_source(c: &Config, dir: &Path) -> Result<Vec<SavedFile>> {
    let mut files = vec![];
    for e in walkdir::WalkDir::new(&c.source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let e = e?;
        if e.depth() == 0 {
            continue;
        }
        let rel = clean_relative(e.path().strip_prefix(&c.source)?)?;
        files.push(saved(&c.source, &rel, dir)?);
    }
    Ok(files)
}
pub fn execute(c: &mut Config, config_path: &Path, p: &Plan, approve: &str) -> Result<Outcome> {
    execute_with_trash(
        c,
        config_path,
        p,
        approve,
        crate::trash_originals::move_to_trash,
    )
}

fn execute_with_trash(
    c: &mut Config,
    config_path: &Path,
    p: &Plan,
    approve: &str,
    mut move_to_trash: impl FnMut(&Path) -> Result<()>,
) -> Result<Outcome> {
    let _lock = Lock::acquire(c)?;
    if p.id != approve || p.id != plan_id(p)? {
        bail!("Approval does not match the exact plan");
    }
    if fingerprint(c, &p.paths, p.action.files())? != p.fingerprint {
        bail!("Files, Git state, or rules changed after review. Create a fresh plan.");
    }
    if p.action == Action::Trash {
        let (targets, digest) =
            crate::trash_originals::prepare(c, &backend::snapshot(c)?, &p.paths)?;
        if targets != p.paths || p.payload.as_deref() != Some(digest.as_str()) {
            bail!(
                "Original files or folder contents changed after review. Create a fresh Trash plan."
            );
        }
    }
    let dir = c.state.join("backups").join(&p.id);
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let source = if matches!(
        p.action,
        Action::Capture
            | Action::Apply
            | Action::Restore
            | Action::Forget
            | Action::EditSource
            | Action::Discard
            | Action::Setting
            | Action::Ignore
            | Action::IgnoreDefaults
    ) {
        backup_source(c, &dir.join("source"))?
    } else {
        vec![]
    };
    let mut home = vec![];
    if p.action.files() && p.action != Action::Trash {
        for path in &p.paths {
            home.push(saved(&c.home, path, &dir.join("home"))?);
        }
    }
    let index = if p.action == Action::Ignore {
        let path = git::index_path(c)?;
        Some(saved(
            path.parent().context("Git index parent missing")?,
            "index",
            &dir.join("git"),
        )?)
    } else {
        None
    };
    let pending = Recovery {
        plan: p.clone(),
        source: source.clone(),
        home: home.clone(),
        index: index.clone(),
        after: String::new(),
        result: Outcome {
            id: p.id.clone(),
            action: p.action,
            succeeded: vec![],
            failed: Some("Interrupted or running; inspect backups before manual recovery".into()),
            not_attempted: p.paths.clone(),
            message: String::new(),
        },
    };
    atomic_write(
        &dir.join("recovery.json"),
        &serde_json::to_vec_pretty(&pending)?,
    )?;
    event(c, p, "running")?;
    let mut result = Outcome {
        id: p.id.clone(),
        action: p.action,
        succeeded: vec![],
        failed: None,
        not_attempted: vec![],
        message: String::new(),
    };
    let per_file = matches!(
        p.action,
        Action::Capture
            | Action::Apply
            | Action::Restore
            | Action::Forget
            | Action::Trash
            | Action::EditSource
    );
    if per_file {
        for (i, path) in p.paths.iter().enumerate() {
            let target = c.target(path)?;
            let r = match p.action {
                Action::Trash => move_to_trash(&target).map(|_| String::new()),
                Action::Capture => backend::chez(
                    c,
                    &[
                        "add",
                        "--recursive=false",
                        "--secrets=error",
                        "--",
                        &target.to_string_lossy(),
                    ],
                ),
                Action::Apply => backend::chez(
                    c,
                    &[
                        "apply",
                        "--recursive=false",
                        "--error-on-conflict",
                        "--",
                        &target.to_string_lossy(),
                    ],
                ),
                Action::Restore => backend::chez(
                    c,
                    &[
                        "apply",
                        "--recursive=false",
                        "--force",
                        "--",
                        &target.to_string_lossy(),
                    ],
                ),
                Action::Forget => {
                    backend::chez(c, &["forget", "--force", "--", &target.to_string_lossy()])
                }
                Action::EditSource => {
                    let src = backend::chez(c, &["source-path", "--", &target.to_string_lossy()])?;
                    let src = Path::new(src.trim());
                    if !src.starts_with(&c.source) {
                        bail!("Source outside repository");
                    }
                    let mode = fs::metadata(src)?.permissions();
                    let r = atomic_write(src, p.payload.as_deref().unwrap_or_default().as_bytes())
                        .and_then(|_| Ok(fs::set_permissions(src, mode)?));
                    r.map(|_| String::new())
                }
                _ => unreachable!(),
            };
            match r {
                Ok(_) => result.succeeded.push(path.clone()),
                Err(e) => {
                    result.failed = Some(path.clone());
                    result.message = e.to_string();
                    result.not_attempted = p.paths[i + 1..].to_vec();
                    break;
                }
            }
        }
    } else {
        let r = execute_group(c, config_path, p);
        match r {
            Ok(msg) => {
                result.succeeded = if p.paths.is_empty() {
                    vec![p.action.label().into()]
                } else {
                    p.paths.clone()
                };
                result.message = msg
            }
            Err(e) => {
                result.failed = Some(p.action.label().into());
                result.message = e.to_string();
            }
        }
    }
    if p.action == Action::Trash && result.failed.is_none() {
        result.message =
            "Originals moved to system Trash. Restore them using your system Trash.".into();
    }
    let after = fingerprint(c, &p.paths, p.action.files())?;
    let recovery = Recovery {
        plan: p.clone(),
        source,
        home,
        index,
        after,
        result: result.clone(),
    };
    atomic_write(
        &dir.join("recovery.json"),
        &serde_json::to_vec_pretty(&recovery)?,
    )?;
    event(
        c,
        p,
        if result.failed.is_some() {
            if result.succeeded.is_empty() {
                "failed"
            } else {
                "partial"
            }
        } else {
            "succeeded"
        },
    )?;
    Ok(result)
}
fn execute_group(c: &mut Config, config_path: &Path, p: &Plan) -> Result<String> {
    let mut args = match p.action {
        Action::Ignore | Action::IgnoreDefaults => {
            return crate::ignores::apply(
                c,
                p.payload.as_deref().context("Ignore change missing")?,
            );
        }
        Action::Stage => process::strings(&["--literal-pathspecs", "add", "--"]),
        Action::Unstage => {
            if git::run(c, &["rev-parse", "HEAD"]).is_ok() {
                process::strings(&["--literal-pathspecs", "restore", "--staged", "--"])
            } else {
                process::strings(&["--literal-pathspecs", "rm", "--cached", "--"])
            }
        }
        Action::Discard => process::strings(&[
            "--literal-pathspecs",
            "restore",
            "--source=HEAD",
            "--worktree",
            "--",
        ]),
        Action::Setting => {
            let change =
                serde_json::from_str(p.payload.as_deref().context("Setting change missing")?)?;
            return crate::settings::update(c, &change, true);
        }
        Action::Commit => {
            return git::run(
                c,
                &["commit", "-m", p.payload.as_deref().unwrap_or_default()],
            );
        }
        Action::Fetch => return git::run(c, &["fetch", "--prune"]),
        Action::Pull => {
            if !git::status(c)?.files.is_empty() {
                bail!("Local source changes block pull");
            }
            return git::run(
                c,
                &[
                    "-c",
                    "rebase.autoStash=false",
                    "-c",
                    "merge.autoStash=false",
                    "pull",
                    "--ff-only",
                    "--no-rebase",
                ],
            );
        }
        Action::Push => {
            let (remote, _, target) = git::push_target(c)?;
            return git::run(c, &["push", &remote, &format!("HEAD:{target}")]);
        }
        Action::Enable | Action::Disable => {
            for path in &p.paths {
                if p.action == Action::Enable {
                    c.enabled.insert(path.clone());
                } else {
                    c.enabled.remove(path);
                }
            }
            c.save(config_path)?;
            return Ok(format!("{} files enabled on this machine", c.enabled.len()));
        }
        _ => bail!("Unsupported grouped action"),
    };
    args.extend(p.paths.clone());
    git::args(c, args)
}
fn event(c: &Config, p: &Plan, outcome: &str) -> Result<()> {
    let e = Event {
        time: chrono::Utc::now(),
        id: p.id.clone(),
        actor: std::env::var("DOT_ACTOR").unwrap_or_else(|_| "user/unspecified".into()),
        intent: std::env::var("DOT_INTENT").unwrap_or_else(|_| p.action.label().into()),
        paths: p.paths.clone(),
        outcome: outcome.into(),
    };
    let mut events = history(c)?;
    events.push(e);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(c.retention_days.into());
    events.retain(|e| e.time >= cutoff);
    atomic_write(
        &c.state.join("activity.json"),
        &serde_json::to_vec_pretty(&events)?,
    )
}
pub fn history(c: &Config) -> Result<Vec<Event>> {
    let p = c.state.join("activity.json");
    if !p.exists() {
        return Ok(vec![]);
    }
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
pub fn recover(c: &Config, id: &str) -> Result<String> {
    if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid recovery id");
    }
    let _lock = Lock::acquire(c)?;
    let dir = c.state.join("backups").join(id);
    let r: Recovery = serde_json::from_slice(&fs::read(dir.join("recovery.json"))?)?;
    if r.plan.action == Action::Trash {
        bail!(
            "Restore moved originals using your system Trash; dot does not restore Trash entries"
        );
    }
    if !matches!(
        r.plan.action,
        Action::Capture
            | Action::Apply
            | Action::Restore
            | Action::Forget
            | Action::EditSource
            | Action::Discard
            | Action::Setting
            | Action::Ignore
            | Action::IgnoreDefaults
    ) {
        bail!("This operation requires Git or scope recovery; file restore is not applicable");
    }
    if r.after.is_empty() {
        bail!(
            "Interrupted operation: backups are preserved at {}. Inspect current files and restore individually; completion state is unknown.",
            dir.display()
        );
    }
    if fingerprint(c, &r.plan.paths, r.plan.action.files())? != r.after {
        bail!(
            "New changes exist since this operation. Preserve them and restore individual backup files manually."
        );
    }
    // Source-wide backup permits recovery of new captures with chezmoi-selected source names.
    // Prune Git before descending. Filtering a contents-first walk can skip later
    // siblings when the deferred .git directory is rejected.
    let existing: Vec<_> = walkdir::WalkDir::new(&c.source)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .collect::<std::result::Result<_, _>>()?;
    for e in existing {
        if e.depth() == 0 || e.path().components().any(|p| p.as_os_str() == ".git") {
            continue;
        }
        let rel = e.path().strip_prefix(&c.source)?.to_string_lossy();
        if !r.source.iter().any(|f| f.relative == rel) && !e.file_type().is_dir() {
            fs::remove_file(e.path())?;
        }
    }
    for f in &r.source {
        restore_file(&c.source, f, &dir.join("source"))?;
    }
    for f in &r.home {
        restore_file(&c.home, f, &dir.join("home"))?;
    }
    if let Some(index) = &r.index {
        let path = git::index_path(c)?;
        restore_file(
            path.parent().context("Git index parent missing")?,
            index,
            &dir.join("git"),
        )?;
    }
    event(c, &r.plan, "restored")?;
    Ok("File backup restored. External script effects are not reversed.".into())
}
fn restore_file(root: &Path, f: &SavedFile, backup: &Path) -> Result<()> {
    clean_relative(Path::new(&f.relative))?;
    let p = root.join(&f.relative);
    if f.kind == "dir" {
        fs::create_dir_all(&p)?;
        return Ok(());
    }
    if fs::symlink_metadata(&p).is_ok() {
        if p.is_dir() && !fs::symlink_metadata(&p)?.file_type().is_symlink() {
            bail!(
                "Refusing to remove directory during file recovery: {}",
                p.display()
            );
        }
        fs::remove_file(&p)?;
    }
    if f.kind == "missing" {
        return Ok(());
    }
    fs::create_dir_all(p.parent().unwrap())?;
    if f.kind == "link" {
        symlink(f.link.as_ref().context("Missing link target")?, &p)?;
    } else {
        fs::copy(backup.join(&f.relative), &p)?;
        fs::set_permissions(&p, fs::Permissions::from_mode(f.mode))?;
    }
    Ok(())
}

#[cfg(test)]
mod trash_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Config, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let c = Config {
            home: temp.path().join("home"),
            source: temp.path().join("source"),
            state: temp.path().join("state"),
            chezmoi_config: Some(temp.path().join("chezmoi.toml")),
            ..Default::default()
        };
        for path in [&c.home, &c.source, &c.state] {
            fs::create_dir_all(path).unwrap();
        }
        fs::write(c.chezmoi_config.as_ref().unwrap(), "").unwrap();
        git::run(&c, &["init", "-b", "main"]).unwrap();
        let config = temp.path().join("dot.toml");
        c.save(&config).unwrap();
        let trash = temp.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        (temp, c, config, trash)
    }

    #[test]
    fn reviewed_trash_moves_only_exact_originals_and_preserves_link_targets() {
        let (temp, mut c, config, trash) = fixture();
        fs::create_dir(c.home.join("scratch")).unwrap();
        fs::write(c.home.join("scratch/keep.txt"), "nested original").unwrap();
        fs::write(c.home.join("loose"), "original").unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "link target").unwrap();
        symlink(&outside, c.home.join("alias")).unwrap();
        let paths = ["scratch", "scratch/keep.txt", "loose", "alias"].map(String::from);
        let p = plan(&c, Action::Trash, &paths, None).unwrap();
        assert_eq!(p.paths, ["alias", "loose", "scratch"]);
        assert!(
            p.preview
                .contains(&c.home.join("scratch").display().to_string())
        );
        assert!(
            c.home.join("scratch/keep.txt").is_file(),
            "planning must not move originals"
        );
        assert!(
            execute_with_trash(&mut c, &config, &p, "wrong", |_| panic!(
                "unapproved Trash call"
            ))
            .is_err()
        );
        let before_git = git::status(&c).unwrap().files;
        let outcome = execute_with_trash(&mut c, &config, &p, &p.id, |path| {
            fs::rename(path, trash.join(path.file_name().unwrap()))?;
            Ok(())
        })
        .unwrap();
        assert!(outcome.failed.is_none());
        assert_eq!(outcome.succeeded, p.paths);
        assert_eq!(
            fs::read_to_string(trash.join("scratch/keep.txt")).unwrap(),
            "nested original"
        );
        assert!(
            fs::symlink_metadata(trash.join("alias"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).unwrap(),
            "link target"
        );
        assert!(!c.home.join("loose").exists());
        assert_eq!(git::status(&c).unwrap().files.len(), before_git.len());
        assert!(
            recover(&c, &p.id)
                .unwrap_err()
                .to_string()
                .contains("system Trash")
        );
    }

    #[test]
    fn trash_rejects_managed_files_mixed_selections_and_changed_folder_contents() {
        let (_temp, mut c, config, _trash) = fixture();
        fs::write(c.source.join("dot_managed"), "saved").unwrap();
        fs::write(c.home.join(".managed"), "original").unwrap();
        fs::create_dir(c.home.join("scratch")).unwrap();
        fs::write(c.home.join("scratch/file"), "before").unwrap();
        for paths in [
            vec![".managed".into()],
            vec!["scratch".into(), ".managed".into()],
        ] {
            let error = plan(&c, Action::Trash, &paths, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("managed"));
        }
        let p = plan(&c, Action::Trash, &["scratch".into()], None).unwrap();
        fs::write(c.home.join("scratch/file"), "changed after review").unwrap();
        let error = execute_with_trash(&mut c, &config, &p, &p.id, |_| panic!("stale Trash call"))
            .unwrap_err();
        assert!(error.to_string().contains("changed after review"));
        assert!(c.home.join("scratch/file").is_file());
        fs::create_dir(c.source.join("scratch")).unwrap();
        fs::write(c.source.join("scratch/file"), "saved now").unwrap();
        assert!(
            plan(&c, Action::Trash, &["scratch".into()], None)
                .unwrap_err()
                .to_string()
                .contains("managed")
        );
    }

    #[test]
    fn trash_refuses_escape_paths_protected_folders_and_managed_empty_directories() {
        let (_temp, mut c, _config, _trash) = fixture();
        c.source = c.home.join("internal/source");
        c.state = c.home.join("internal/state");
        fs::create_dir_all(&c.source).unwrap();
        fs::create_dir_all(&c.state).unwrap();
        fs::create_dir(c.home.join(".git")).unwrap();
        fs::create_dir(c.home.join("empty")).unwrap();
        symlink(&c.source, c.home.join("link")).unwrap();
        let mut snapshot = backend::Snapshot::default();
        snapshot.managed_directories.insert("empty".into());
        for path in [
            "",
            "../escape",
            "internal",
            "internal/source",
            "internal/state",
            ".git",
            "empty",
            "link/child",
        ] {
            assert!(
                crate::trash_originals::prepare(&c, &snapshot, &[path.into()]).is_err(),
                "allowed {path}"
            );
        }
    }

    #[test]
    fn trash_stops_after_a_failure_and_reports_unattempted_originals() {
        let (_temp, mut c, config, trash) = fixture();
        for name in ["first", "second", "third"] {
            fs::write(c.home.join(name), name).unwrap();
        }
        let p = plan(
            &c,
            Action::Trash,
            &["first".into(), "second".into(), "third".into()],
            None,
        )
        .unwrap();
        let outcome = execute_with_trash(&mut c, &config, &p, &p.id, |path| {
            if path.file_name().unwrap() == "second" {
                bail!("fixture Trash unavailable");
            }
            fs::rename(path, trash.join(path.file_name().unwrap()))?;
            Ok(())
        })
        .unwrap();
        assert_eq!(outcome.succeeded, ["first"]);
        assert_eq!(outcome.failed.as_deref(), Some("second"));
        assert_eq!(outcome.not_attempted, ["third"]);
        assert!(outcome.message.contains("Trash unavailable"));
        assert!(trash.join("first").is_file());
        assert!(c.home.join("second").is_file());
        assert!(c.home.join("third").is_file());
    }
}
