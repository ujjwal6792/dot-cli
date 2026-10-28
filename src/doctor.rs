//! `dw doctor`: tool versions, paths, machine and repository health.

use serde::Serialize;

use crate::assign;
use crate::exec;
use crate::paths::tilde;
use crate::pins;
use crate::status;
use crate::store::Store;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    /// A finding worth showing that does not make the report fail.
    pub warning: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub ok: bool,
    pub checks: Vec<Check>,
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        ok,
        warning: false,
        detail: detail.into(),
    }
}

fn tool_version(
    name: &str,
    mut cmd: std::process::Command,
    want: &str,
    path: &std::path::Path,
) -> Check {
    cmd.arg("--version");
    match exec::run(cmd, None) {
        Ok(o) if o.success() => {
            let line = o.stdout.lines().next().unwrap_or("").trim().to_string();
            let ok = line.contains(want);
            let detail = if ok {
                format!("{line} ({})", path.display())
            } else {
                format!("{line} ({}): expected version {want}", path.display())
            };
            check(name, ok, detail)
        }
        Ok(o) => check(
            name,
            false,
            format!(
                "{} exited with {}: {}",
                path.display(),
                o.status,
                o.stderr.trim()
            ),
        ),
        Err(e) => check(name, false, format!("{}: {e:#}", path.display())),
    }
}

pub fn run(store: &Store) -> Report {
    let mut checks = Vec::new();
    let t = &store.tools;
    let d = &store.dirs;

    checks.push(check(
        "libexec",
        t.libexec().is_dir(),
        t.libexec().display().to_string(),
    ));
    checks.push(tool_version(
        "chezmoi",
        std::process::Command::new(t.chezmoi()),
        pins::chezmoi_version(),
        &t.chezmoi(),
    ));
    checks.push(tool_version(
        "git",
        store.git_command(&d.home),
        pins::git_version(),
        &t.git_bin(),
    ));
    checks.push(tool_version(
        "delta",
        std::process::Command::new(t.delta()),
        pins::delta_version(),
        &t.delta(),
    ));

    let name = store.git_identity_value("user.name");
    let email = store.git_identity_value("user.email");
    checks.push(check(
        "git identity",
        name.is_some() && email.is_some(),
        match (name, email) {
            (Some(n), Some(e)) => format!("{n} <{e}> (from the global git config)"),
            _ => "user.name/user.email missing from the global git config".into(),
        },
    ));

    for (label, path) in [
        ("repository", d.repo()),
        ("chezmoi config", d.chezmoi_config()),
        ("chezmoi state", d.chezmoi_state()),
        ("machine.toml", d.machine_toml()),
    ] {
        // Informational: these legitimately do not exist before `dw init`.
        let exists = path.exists();
        checks.push(check(
            &format!("path: {label}"),
            true,
            format!(
                "{}{}",
                tilde(&d.home, &path),
                if exists { "" } else { " (not created yet)" }
            ),
        ));
    }

    if !store.is_initialised() {
        checks.push(check(
            "repository state",
            true,
            "not initialised (run `dw init` or `dw clone <url>`)",
        ));
    } else {
        match status::collect(store, None) {
            Ok(s) => {
                checks.push(match &s.machine {
                    Some(m) if m.registered.is_some() => check(
                        "machine",
                        true,
                        format!("{} is in the machine registry", m.name),
                    ),
                    Some(m) => check("machine", false, m.problem().unwrap_or_default()),
                    None => check(
                        "machine",
                        false,
                        "this computer is not registered (machine.toml missing)",
                    ),
                });
                if let Some(r) = &s.repo {
                    checks.push(check(
                        "repository state",
                        r.changed_paths == 0,
                        format!(
                            "branch {} @ {}, {} uncommitted path(s)",
                            r.branch.as_deref().unwrap_or("?"),
                            r.head.as_deref().unwrap_or("no commits"),
                            r.changed_paths
                        ),
                    ));
                }
                let expected = s
                    .machine
                    .as_ref()
                    .and_then(|m| Store::chezmoi_config_body(&m.name).ok());
                let on_disk = std::fs::read_to_string(d.chezmoi_config()).ok();
                let config_ok = expected.is_some() && expected == on_disk;
                checks.push(check(
                    "chezmoi config",
                    config_ok,
                    if config_ok {
                        "matches the registered machine".to_string()
                    } else {
                        "missing or stale; it is regenerated when the machine is registered"
                            .to_string()
                    },
                ));
                match assign::unknown_targets(&store.repo()) {
                    Ok(unknown) if !unknown.is_empty() => {
                        let mut c = check(
                            "targets",
                            true,
                            format!("unknown target kinds never match: {}", unknown.join("; ")),
                        );
                        c.warning = true;
                        checks.push(c);
                    }
                    Ok(_) => {}
                    Err(e) => checks.push(check("targets", false, format!("{e:#}"))),
                }
                match crate::variants::clashes(store) {
                    Ok(c) if !c.is_empty() => {
                        let list: Vec<String> =
                            c.iter().map(|(p, why)| format!("~/{p}: {why}")).collect();
                        let mut w = check(
                            "variants",
                            true,
                            format!("blocked until resolved: {}", list.join("; ")),
                        );
                        w.warning = true;
                        checks.push(w);
                    }
                    Ok(_) => {}
                    Err(e) => checks.push(check("variants", false, format!("{e:#}"))),
                }
                if config_ok {
                    checks.push(match store.chezmoi(&["managed"]) {
                        Ok(o) => check(
                            "chezmoi templates",
                            true,
                            format!(
                                "{} target(s) apply to this machine",
                                o.stdout.lines().count()
                            ),
                        ),
                        Err(e) => check("chezmoi templates", false, format!("{e:#}")),
                    });
                }
            }
            Err(e) => checks.push(check("repository state", false, format!("{e:#}"))),
        }
    }

    Report {
        ok: checks.iter().all(|c| c.ok),
        checks,
    }
}

impl Report {
    pub fn render_human(&self) -> String {
        let mut s = String::new();
        for c in &self.checks {
            s.push_str(&format!(
                "[{}] {:<18} {}\n",
                if c.ok { "ok" } else { "FAIL" },
                c.name,
                c.detail
            ));
        }
        s
    }
}
