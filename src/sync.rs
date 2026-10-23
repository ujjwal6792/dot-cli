use crate::{
    backend,
    config::{Config, atomic_write},
    git,
    operations::{self, Action, Lock},
    process,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub installation: String,
    pub revision: String,
    pub observed_at: String,
    pub outcome: String,
    pub enabled: usize,
    pub updated: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub outcome: String,
    pub message: String,
    pub updated: usize,
    pub report_upload: Option<String>,
}

/// Conservative eligibility is checked BEFORE invoking a command that reads chezmoi source.
/// This is a restricted automatic mode, not a sandbox for arbitrary repository code.
pub fn strict_check(c: &Config) -> Result<()> {
    let version = process::run(&c.chezmoi, &process::strings(&["--version"]), None, None)?;
    if !version
        .split_whitespace()
        .any(|s| s.trim_end_matches(',') == "v2.73.1" || s.trim_end_matches(',') == "2.73.1")
    {
        bail!(
            "Strict automatic eligibility is verified for chezmoi v2.73.1; review this tool version before unattended execution"
        );
    }
    let configs = crate::config::native_config_paths(c);
    for path in configs.into_iter().filter(|p| p.exists()) {
        let text = fs::read_to_string(&path)?;
        let keys: Vec<String> = if path.extension().is_some_and(|x| x == "toml") {
            toml::from_str::<toml::Table>(&text)?
                .keys()
                .cloned()
                .collect()
        } else if path.extension().is_some_and(|x| x == "json") {
            serde_json::from_str::<serde_json::Value>(&text)?
                .as_object()
                .context("Config must be an object")?
                .keys()
                .cloned()
                .collect()
        } else {
            bail!("Strict sync requires reviewed TOML/JSON config; YAML needs trusted mode");
        };
        if keys.iter().any(|k| {
            !matches!(
                k.as_str(),
                "sourceDir" | "destDir" | "cacheDir" | "persistentState" | "data"
            )
        }) {
            bail!(
                "Chezmoi config has behavior outside strict sync (hooks, integrations or other settings); review or opt into trusted mode"
            );
        }
    }
    for e in walkdir::WalkDir::new(&c.source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let e = e?;
        if e.depth() == 0 {
            continue;
        }
        let name = e.file_name().to_string_lossy();
        if e.file_type().is_symlink()
            || name.ends_with(".tmpl")
            || name.starts_with(".chezmoi")
            || [
                "run_",
                "modify_",
                "remove_",
                "exact_",
                "external_",
                "symlink_",
                "encrypted_",
                "executable_",
                "create_",
            ]
            .iter()
            .any(|p| name.starts_with(p) || name.contains(&format!("_{p}")))
        {
            bail!(
                "Strict sync paused for dynamic/special source: {}. Review or use trusted mode.",
                e.path().strip_prefix(&c.source)?.display()
            );
        }
    }
    Ok(())
}
#[derive(Debug)]
struct PartialApply {
    updated: usize,
    message: String,
}
impl std::fmt::Display for PartialApply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} files updated before failure: {}",
            self.updated, self.message
        )
    }
}
impl std::error::Error for PartialApply {}

pub fn once(c: &mut Config, config_path: &Path) -> Result<SyncResult> {
    if !c.sync_enabled {
        bail!("Automatic sync is disabled on this machine");
    }
    let result = sync_inner(c, config_path);
    let mut out = match result {
        Ok(n) => SyncResult {
            outcome: "succeeded".into(),
            message: "Configuration synchronized".into(),
            updated: n,
            report_upload: None,
        },
        Err(e) => SyncResult {
            outcome: if e.downcast_ref::<PartialApply>().is_some() {
                "partial"
            } else {
                "paused"
            }
            .into(),
            message: e.to_string(),
            updated: e.downcast_ref::<PartialApply>().map_or(0, |e| e.updated),
            report_upload: None,
        },
    };
    let report = Report {
        installation: c.installation.clone(),
        revision: git::run(c, &["rev-parse", "HEAD"])
            .unwrap_or_default()
            .trim()
            .into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
        outcome: out.outcome.clone(),
        enabled: c.enabled.len(),
        updated: out.updated,
    };
    atomic_write(
        &c.state.join("last-report.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    if c.reporting {
        let previous: Option<Report> = fs::read(c.state.join("published-report.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let unchanged = previous.as_ref().is_some_and(|p| {
            p.revision == report.revision
                && p.outcome == report.outcome
                && p.enabled == report.enabled
                && p.updated == report.updated
                && chrono::DateTime::parse_from_rfc3339(&p.observed_at)
                    .is_ok_and(|t| chrono::Utc::now().signed_duration_since(t).num_hours() < 1)
        });
        out.report_upload = Some(if unchanged {
            "unchanged; retaining last published observation".into()
        } else {
            match publish(c, &report) {
                Ok(_) => "published".into(),
                Err(e) => format!("upload failed: {e}"),
            }
        });
    }
    atomic_write(
        &c.state.join("last-sync.json"),
        &serde_json::to_vec_pretty(&out)?,
    )?;
    Ok(out)
}
fn sync_inner(c: &mut Config, config_path: &Path) -> Result<usize> {
    if c.enabled.is_empty() {
        return Ok(0);
    }
    if !c.trusted {
        strict_check(c)?;
    }
    let before = backend::snapshot(c)?;
    if !before.git.files.is_empty() {
        bail!("Source has local changes; automatic pull paused");
    }
    for p in &c.enabled {
        if before.scoped_out.contains_key(p) {
            continue;
        }
        if !c.target(p)?.is_file() {
            bail!("New/missing or non-file target requires review: {p}");
        }
        if before.changes.contains_key(p) {
            bail!("Local changes require review: {p}");
        }
    }
    let p = operations::plan(c, Action::Pull, &[], None)?;
    let r = operations::execute(c, config_path, &p, &p.id)?;
    if r.failed.is_some() {
        bail!("Pull paused: {}", r.message);
    }
    if !c.trusted {
        strict_check(c)?;
    }
    let after = backend::snapshot(c)?;
    for path in &c.enabled {
        if after.scoped_out.contains_key(path) {
            continue;
        }
        if before.scoped_out.contains_key(path) && after.managed.contains_key(path) {
            bail!("File now applies to this machine; review before automatic application: {path}");
        }
        if !after.managed.contains_key(path) {
            bail!("Enabled source removed; review deletion: {path}");
        }
    }
    let paths: Vec<_> = c
        .enabled
        .iter()
        .filter(|p| after.changes.contains_key(*p))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Ok(0);
    }
    let p = operations::plan(c, Action::Apply, &paths, None)?;
    let r = operations::execute(c, config_path, &p, &p.id)?;
    if r.failed.is_some() {
        return Err(PartialApply {
            updated: r.succeeded.len(),
            message: r.message,
        }
        .into());
    }
    Ok(r.succeeded.len())
}
pub fn publish(c: &Config, r: &Report) -> Result<()> {
    if !c.reporting {
        bail!("Report publishing is not enabled");
    }
    let _lock = Lock::acquire(c)?;
    uuid::Uuid::parse_str(&c.installation).context("Invalid installation identity")?;
    let branch = format!("refs/heads/dot-reports/{}", c.installation);
    let remote = git::run(c, &["ls-remote", "origin", &branch])?;
    let old = remote.split_whitespace().next().unwrap_or("").to_string();
    if !old.is_empty() {
        git::run(
            c,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "origin",
                &branch,
            ],
        )?;
        let previous = git::run(c, &["show", &format!("{old}:report.json")])?;
        let previous: Report = serde_json::from_str(&previous)?;
        if previous.installation != c.installation {
            bail!("Report identity collision; refusing publication");
        }
    }
    let blob = process::run(
        &c.git,
        &process::strings(&["hash-object", "-w", "--stdin"]),
        Some(&c.source),
        Some(&serde_json::to_vec_pretty(r)?),
    )?;
    let tree = process::run(
        &c.git,
        &process::strings(&["mktree"]),
        Some(&c.source),
        Some(format!("100644 blob {}\treport.json\n", blob.trim()).as_bytes()),
    )?;
    let mut a = process::strings(&[
        "-c",
        "user.name=dot status reporter",
        "-c",
        "user.email=dot@localhost",
        "-c",
        "commit.gpgsign=false",
        "commit-tree",
        tree.trim(),
        "-m",
        "Machine status report",
    ]);
    if !old.is_empty() {
        a.extend(["-p".into(), old.clone()]);
    }
    let commit = git::args(c, a)?;
    git::run(
        c,
        &[
            "push",
            &format!("--force-with-lease={branch}:{old}"),
            "origin",
            &format!("{}:{branch}", commit.trim()),
        ],
    )?;
    atomic_write(
        &c.state.join("published-report.json"),
        &serde_json::to_vec_pretty(r)?,
    )?;
    Ok(())
}
pub fn fetch_reports(c: &Config) -> Result<Vec<Report>> {
    let _lock = Lock::acquire(c)?;
    git::run(
        c,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "origin",
            "+refs/heads/dot-reports/*:refs/dot/reports/*",
        ],
    )?;
    let refs = git::run(
        c,
        &["for-each-ref", "--format=%(refname)", "refs/dot/reports/"],
    )?;
    let mut reports = vec![];
    for r in refs.lines() {
        let text = git::run(c, &["show", &format!("{r}:report.json")])?;
        reports.push(serde_json::from_str::<Report>(&text)?);
    }
    atomic_write(
        &c.state.join("reports.json"),
        &serde_json::to_vec_pretty(&reports)?,
    )?;
    atomic_write(
        &c.state.join("reports-fetched-at"),
        chrono::Utc::now().to_rfc3339().as_bytes(),
    )?;
    Ok(reports)
}
pub fn cached_reports(c: &Config) -> Vec<Report> {
    fs::read(c.state.join("reports.json"))
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default()
}
pub fn service_file(c: &Config, config: &Path) -> Result<(PathBuf, String)> {
    let exe = std::env::current_exe()?;
    let config = std::path::absolute(config)?;
    if cfg!(target_os = "macos") {
        fn xml(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        }
        Ok((
            c.home.join("Library/LaunchAgents/io.dot-cli.sync.plist"),
            format!(
                "<?xml version=\"1.0\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>io.dot-cli.sync</string><key>ProgramArguments</key><array><string>{}</string><string>--config</string><string>{}</string><string>sync</string><string>--watch</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/></dict></plist>\n",
                xml(&exe.to_string_lossy()),
                xml(&config.to_string_lossy())
            ),
        ))
    } else {
        fn quote(s: &str) -> String {
            format!(
                "\"{}\"",
                s.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('%', "%%")
                    .replace('\n', "")
            )
        }
        Ok((
            c.home.join(".config/systemd/user/dot-sync.service"),
            format!(
                "[Unit]\nDescription=dot configuration synchronization\n[Service]\nExecStart={} --config {} sync --watch\nRestart=on-failure\nRestartSec=30\n[Install]\nWantedBy=default.target\n",
                quote(&exe.to_string_lossy()),
                quote(&config.to_string_lossy())
            ),
        ))
    }
}

pub fn install_service(c: &Config, config: &Path, start: bool) -> Result<String> {
    if !c.sync_enabled {
        bail!("Enable automatic sync on this machine before installing a service");
    }
    let (path, body) = service_file(c, config)?;
    if path.exists() {
        let previous = fs::read_to_string(&path)?;
        if previous != body {
            let backup = c
                .state
                .join("service-backups")
                .join(uuid::Uuid::new_v4().to_string());
            atomic_write(&backup, previous.as_bytes())?;
        }
    }
    atomic_write(&path, body.as_bytes())?;
    if start {
        if cfg!(target_os = "macos") {
            let uid = process::run("id", &process::strings(&["-u"]), None, None)?;
            process::run(
                "launchctl",
                &process::strings(&[
                    "bootstrap",
                    &format!("gui/{}", uid.trim()),
                    &path.to_string_lossy(),
                ]),
                None,
                None,
            )?;
        } else {
            process::run(
                "systemctl",
                &process::strings(&["--user", "daemon-reload"]),
                None,
                None,
            )?;
            process::run(
                "systemctl",
                &process::strings(&["--user", "enable", "--now", "dot-sync.service"]),
                None,
                None,
            )?;
        }
    }
    Ok(format!(
        "Installed {}{}",
        path.display(),
        if start {
            " and requested user-service activation"
        } else {
            "; service has not been started"
        }
    ))
}
