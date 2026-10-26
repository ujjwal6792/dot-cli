//! Command-line interface (spec §9).

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::json;

use crate::error::{self, DwError};
use crate::machine;
use crate::repo::{self, DEFAULT_BRANCH};
use crate::store::Store;
use crate::{doctor, status, tui};

/// Dotfiles, secrets and variables in sync across your machines.
///
/// Exit codes: 0 ok, 1 error, 2 usage, 3 needs approval, 4 vault locked,
/// 5 conflict, 6 refused public remote, 7 stale. With --json, errors are printed
/// on stdout as {"error": {"kind", "code", "message"}}.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Print machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new Dotwarden repository and register this computer as a machine.
    Init {
        /// Registry name for this machine (a-z, 0-9, '-'); prompted for when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Name of the default branch of the new repository.
        #[arg(long, default_value = DEFAULT_BRANCH)]
        branch: String,
        /// Show exactly what would be created without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Clone an existing Dotwarden repository and register this computer as a machine.
    Clone {
        /// Location of the repository: an https:// or ssh URL, a file:// URL or a path.
        url: String,
        /// Registry name for this machine (a-z, 0-9, '-'); prompted for when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Reuse an existing registry entry with this name (re-installing the same machine).
        #[arg(long)]
        adopt: bool,
        /// Show exactly what would be done without cloning or changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show this machine, the repository and the pending list.
    Status,
    /// Check tool versions, paths, the registered machine and repository health.
    Doctor,
    /// Open the terminal UI (the default when no command is given).
    Tui,
}

pub fn main(bin_name: &'static str) -> ExitCode {
    let json = std::env::args_os().any(|a| a == "--json");
    let matches = match Cli::command().name(bin_name).try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let informational =
                matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            if informational || !json {
                e.exit();
            }
            return report(error::Kind::Usage, &e.render().to_string(), true);
        }
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    let json = cli.json;
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            let (kind, message) = error::classify(&e);
            report(kind, &message, json)
        }
    }
}

fn report(kind: error::Kind, message: &str, json: bool) -> ExitCode {
    if json {
        println!("{}", error::to_json(kind, message));
    } else {
        eprintln!("error: {}", message.trim_end());
    }
    ExitCode::from(kind.code())
}

fn run(cli: Cli) -> Result<ExitCode> {
    let store = Store::from_process()?;
    let json = cli.json;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Init {
            name,
            branch,
            dry_run,
        } => {
            let name = resolve_name(name, &[], dry_run)?;
            cmd_init(&store, name, &branch, dry_run, json)?;
        }
        Command::Clone {
            url,
            name,
            adopt,
            dry_run,
        } => cmd_clone(&store, &url, name, adopt, dry_run, json)?,
        Command::Status => {
            let s = status::collect(&store, None)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                print!("{}", s.render_human());
            }
        }
        Command::Doctor => {
            let report = doctor::run(&store);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_human());
            }
            if !report.ok {
                return Ok(ExitCode::from(error::Kind::Error.code()));
            }
        }
        Command::Tui => {
            if !store.is_initialised() {
                return Err(DwError::usage(
                    "not initialised; run `dw init` or `dw clone <url>` first",
                )
                .into());
            }
            tui::run(store)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Use `--name` when given; otherwise prompt on a terminal, or fail. With
/// `dry_run` nothing is prompted: no name stays `None`.
fn resolve_name(given: Option<String>, taken: &[String], dry_run: bool) -> Result<Option<String>> {
    if let Some(n) = given {
        machine::validate_name(&n)?;
        return Ok(Some(n));
    }
    if dry_run {
        return Ok(None);
    }
    if !std::io::stdin().is_terminal() {
        return Err(DwError::usage(
            "no machine name given; pass --name <name> (stdin is not a terminal)",
        )
        .into());
    }
    prompt_name(taken).map(Some)
}

fn prompt_name(taken: &[String]) -> Result<String> {
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_default();
    let suggestion = machine::suggest_name(&host);
    if !taken.is_empty() {
        eprintln!("Machines already registered: {}", taken.join(", "));
    }
    let stdin = std::io::stdin();
    loop {
        eprint!("Name for this machine [{suggestion}]: ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            return Err(DwError::usage("no machine name given").into());
        }
        let name = match line.trim() {
            "" => suggestion.clone(),
            other => other.to_string(),
        };
        match machine::validate_name(&name) {
            Ok(()) => return Ok(name),
            Err(e) => eprintln!("{e}"),
        }
    }
}

fn describe_machine(m: &machine::Machine) -> String {
    format!(
        "{} ({}/{}{})",
        m.name,
        m.platform,
        m.distribution,
        if m.families.is_empty() {
            String::new()
        } else {
            format!(", family {}", m.families.join(", "))
        }
    )
}

fn report_registration(
    store: &Store,
    reg: &repo::Registration,
    verb: &str,
    json: bool,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "repository": store.repo(),
                "machine": reg.machine,
                "registered": reg.created,
            }))?
        );
    } else {
        println!("{verb} repository at {}", store.repo().display());
        println!(
            "{} machine {}",
            if reg.created { "Registered" } else { "Adopted" },
            describe_machine(&reg.machine)
        );
    }
    Ok(())
}

fn print_plan(plan: &serde_json::Value, human: &[String], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
    } else {
        println!("Dry run: nothing was changed. Would:");
        for line in human {
            println!("  - {line}");
        }
    }
    Ok(())
}

fn cmd_init(
    store: &Store,
    name: Option<String>,
    branch: &str,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    if dry_run {
        repo::require_empty_repo_location(store)?;
        repo::require_git_identity(store)?;
        let detected = machine::detect()?;
        let files = repo::layout_paths();
        let machine = name.as_deref().map(|n| detected.clone().into_machine(n));
        let plan = json!({
            "dry_run": true,
            "command": "init",
            "repository": store.repo(),
            "branch": branch,
            "files": files,
            "commits": ["Initialise Dotwarden repository", "Register machine"],
            "machine": machine,
            "machine_name": if machine.is_none() { json!("prompted for") } else { json!(null) },
            "writes": [store.dirs.chezmoi_config(), store.dirs.machine_toml()],
        });
        let mut human = vec![format!(
            "create the repository at {} on branch {branch}",
            store.repo().display()
        )];
        human.extend(files.iter().map(|f| format!("write {f}")));
        human.push(match &machine {
            Some(m) => format!("register machine {}", describe_machine(m)),
            None => format!(
                "register this computer ({}/{}) under a name you are prompted for",
                detected.platform, detected.distribution
            ),
        });
        human.push(format!("write {}", store.dirs.chezmoi_config().display()));
        human.push(format!("write {}", store.dirs.machine_toml().display()));
        return print_plan(&plan, &human, json);
    }
    let name = name.expect("a name is always resolved outside dry runs");
    let _lock = store.lock()?;
    repo::init(store, branch)?;
    match repo::register_machine(store, &name, false) {
        Ok(reg) => report_registration(store, &reg, "Initialised", json),
        Err(e) => {
            let _ = std::fs::remove_dir_all(store.repo());
            Err(e)
        }
    }
}

fn cmd_clone(
    store: &Store,
    url: &str,
    name: Option<String>,
    adopt: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    if let Some(n) = &name {
        machine::validate_name(n)?;
    } else if !dry_run && !std::io::stdin().is_terminal() {
        return Err(DwError::usage(
            "no machine name given; pass --name <name> (stdin is not a terminal)",
        )
        .into());
    }
    if dry_run {
        repo::require_empty_repo_location(store)?;
        repo::require_git_identity(store)?;
        let detected = machine::detect()?;
        let machine = name.as_deref().map(|n| detected.clone().into_machine(n));
        let plan = json!({
            "dry_run": true,
            "command": "clone",
            "url": url,
            "repository": store.repo(),
            "adopt": adopt,
            "machine": machine,
            "machine_name": if machine.is_none() { json!("prompted for") } else { json!(null) },
            "commits": ["Register machine"],
            "writes": [store.dirs.chezmoi_config(), store.dirs.machine_toml()],
            "note": "the registry of the remote is only known after cloning, so name conflicts are not checked",
        });
        let human = vec![
            format!("clone {url} into {}", store.repo().display()),
            match &machine {
                Some(m) => format!(
                    "register machine {}{}",
                    describe_machine(m),
                    if adopt {
                        " (adopting an existing entry)"
                    } else {
                        ""
                    }
                ),
                None => "register this computer under a name you are prompted for".to_string(),
            },
            format!("write {}", store.dirs.chezmoi_config().display()),
            format!("write {}", store.dirs.machine_toml().display()),
            "name conflicts with the remote registry are only checked after cloning".to_string(),
        ];
        return print_plan(&plan, &human, json);
    }
    let _lock = store.lock()?;
    repo::clone(store, url)?;
    let result = (|| {
        let name = match name {
            Some(n) => n,
            None => {
                let taken: Vec<String> = repo::read_registry(&store.repo())?
                    .into_iter()
                    .map(|m| m.name)
                    .collect();
                resolve_name(None, &taken, false)?.expect("prompted")
            }
        };
        repo::register_machine(store, &name, adopt)
    })();
    match result {
        Ok(reg) => report_registration(store, &reg, "Cloned", json),
        Err(e) => {
            let _ = std::fs::remove_dir_all(store.repo());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(cmd: &clap::Command, path: String, problems: &mut Vec<String>) {
        for arg in cmd.get_arguments() {
            if arg.get_help().is_none() && arg.get_id() != "help" && arg.get_id() != "version" {
                problems.push(format!(
                    "{path}: argument `{}` has no help text",
                    arg.get_id()
                ));
            }
        }
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let p = format!("{path} {}", sub.get_name());
            if sub.get_about().is_none() {
                problems.push(format!("{p}: no description"));
            }
            walk(sub, p, problems);
        }
    }

    #[test]
    fn every_command_flag_and_argument_has_help_text() {
        let mut problems = Vec::new();
        walk(&Cli::command(), "dotwarden".into(), &mut problems);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn json_is_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["dw", "status", "--json"]).unwrap();
        assert!(cli.json);
    }
}
