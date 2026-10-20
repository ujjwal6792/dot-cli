use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use dot_cli::{
    backend,
    config::Config,
    migration,
    operations::{self, Action, Plan},
    process, settings, sync, ui,
};
use std::{fs, path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(
    name = "dot",
    version,
    about = "A unified chezmoi + Git workspace",
    long_about = "Manage dotfiles through chezmoi, review and publish through Git, and observe machine activity. Run without a command for the Ratatui workspace."
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[arg(long, global = true)]
    source: Option<PathBuf>,
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    chezmoi_config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// Connect an ordinary chezmoi repository and install the small agent skill. Does not apply files.
    Init {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        machine: Option<String>,
    },
    /// Inspect managed destinations and Git state as JSON.
    Status,
    /// Inspect the native chezmoi diff. --capture reverses it to HOME → source.
    Diff {
        paths: Vec<String>,
        #[arg(long)]
        capture: bool,
    },
    /// Freeze an exact review and print its JSON, including approval id.
    Plan {
        #[arg(value_enum)]
        action: Action,
        paths: Vec<String>,
        #[arg(short = 'm', long)]
        message: Option<String>,
        #[arg(long)]
        content_file: Option<PathBuf>,
    },
    /// Execute a saved, unchanged plan after explicit review.
    Execute {
        id: String,
        #[arg(long)]
        approve: String,
    },
    /// Show or change local automatic-update policy. Enablement uses plan enable/disable.
    Settings {
        key: Option<String>,
        value: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Synchronize once, or keep a worker running. Requires local opt-in.
    Sync {
        #[arg(long)]
        watch: bool,
    },
    /// Inspect cached machine reports, fetch them, or publish the local summary.
    Reports {
        #[arg(long)]
        fetch: bool,
        #[arg(long)]
        publish: bool,
    },
    /// Read the local user/agent action journal.
    Activity,
    /// List recovery snapshots or restore one when no intervening changes exist.
    Recovery {
        id: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Generate/install a local launchd or systemd service (activation stays explicit).
    Service {
        #[arg(long)]
        install: bool,
        #[arg(long, requires = "install")]
        start: bool,
    },
    /// Preview or convert an old catalog into a NEW chezmoi source directory.
    Migrate {
        old: PathBuf,
        destination: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Archive and switch to another existing source repository, preserving HOME.
    Source {
        path: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Review/update native chezmoi data and template helper for shared settings.
    Setting {
        key: Option<String>,
        value: Option<String>,
        #[arg(long, value_enum, default_value = "shared")]
        scope: settings::Scope,
        #[arg(long)]
        remove: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Check runtime tools and paths without installing anything.
    Health,
    /// List dependency status or explicitly install missing Git/chezmoi with the system manager.
    Packages {
        #[arg(long)]
        install: bool,
    },
}
fn main() {
    if let Err(e) = entry() {
        eprintln!("dot: {e:#}");
        std::process::exit(1);
    }
}
fn entry() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(Config::default_path);
    let mut c = Config::load(&path)?;
    if let Some(v) = cli.home {
        c.home = v;
    }
    if let Some(v) = cli.source {
        c.source = v;
    }
    if let Some(v) = cli.state_dir {
        c.state = v;
    }
    if let Some(v) = cli.chezmoi_config {
        c.chezmoi_config = Some(v);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Ok(exe) = exe.canonicalize()
        && let Some(parent) = exe.parent()
    {
        let private = parent.join("libexec/chezmoi");
        if private.is_file() {
            c.chezmoi = private.to_string_lossy().into_owned();
        }
    }
    c.resolve_tools();
    c.validate()?;
    if !path.exists()
        && matches!(
            &cli.command,
            None | Some(Commands::Plan { .. }) | Some(Commands::Setting { .. })
        )
    {
        c.save(&path)?;
    }
    match cli.command {
        None => ui::run(c, path),
        Some(Commands::Init { repo, machine }) => {
            if let Some(m) = machine {
                if m.trim().is_empty() {
                    bail!("Machine name must not be empty");
                }
                c.machine = m;
            }
            let report = ui::initialize(&c, &path, repo.as_deref())?;
            println!("{}", report.message(&c.source));
            Ok(())
        }
        Some(Commands::Status) => print(&backend::snapshot(&c)?),
        Some(Commands::Diff { paths, capture }) => {
            let s = backend::snapshot(&c)?;
            let p = if paths.is_empty() {
                s.managed.keys().cloned().collect()
            } else {
                backend::exact_targets(&c, &s, &paths, false)?
            };
            println!("{}", backend::preview(&c, &p, capture)?);
            Ok(())
        }
        Some(Commands::Plan {
            action,
            paths,
            message,
            content_file,
        }) => {
            let payload = if let Some(file) = content_file {
                Some(fs::read_to_string(file)?)
            } else {
                message
            };
            print(&operations::plan(&c, action, &paths, payload)?)
        }
        Some(Commands::Execute { id, approve }) => {
            if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("Invalid plan id");
            }
            let plan: Plan = serde_json::from_slice(&fs::read(
                c.state.join("plans").join(format!("{id}.json")),
            )?)?;
            let out = operations::execute(&mut c, &path, &plan, &approve)?;
            print(&out)?;
            if out.failed.is_some() {
                bail!("Operation failed or partially completed; see result and dot recovery");
            }
            Ok(())
        }
        Some(Commands::Settings { key, value, yes }) => {
            if let Some(key) = key {
                if !yes {
                    bail!("Review dot settings, then use --yes for this local policy change");
                }
                let val = value.context("Value is required")?;
                let _lock = operations::Lock::acquire(&c)?;
                match key.as_str() {
                    "sync" => c.sync_enabled = val.parse()?,
                    "trusted" => c.trusted = val.parse()?,
                    "reporting" => c.reporting = val.parse()?,
                    "interval" => c.interval_seconds = val.parse()?,
                    "retention" => c.retention_days = val.parse()?,
                    "machine" => c.machine = val,
                    _ => bail!("Keys: sync, trusted, reporting, interval, retention, machine"),
                };
                c.validate()?;
                c.save(&path)?;
            }
            print(&c)
        }
        Some(Commands::Sync { watch }) => {
            let mut failures = 0u32;
            loop {
                if watch {
                    c = Config::load(&path)?;
                    c.validate()?;
                }
                if watch && !c.sync_enabled {
                    std::thread::sleep(Duration::from_secs(c.interval_seconds));
                    continue;
                }
                let out = sync::once(&mut c, &path)?;
                failures = if out.outcome == "succeeded" {
                    0
                } else {
                    (failures + 1).min(4)
                };
                print(&out)?;
                if !watch {
                    if out.outcome != "succeeded" {
                        bail!("Sync paused");
                    }
                    break;
                }
                std::thread::sleep(Duration::from_secs(
                    (c.interval_seconds.saturating_mul(1 << failures)).min(3600),
                ));
            }
            Ok(())
        }
        Some(Commands::Reports { fetch, publish }) => {
            if fetch {
                return print(&sync::fetch_reports(&c)?);
            }
            if publish {
                let r: sync::Report =
                    serde_json::from_slice(&fs::read(c.state.join("last-report.json"))?)?;
                sync::publish(&c, &r)?;
            }
            print(&sync::cached_reports(&c))
        }
        Some(Commands::Activity) => print(&operations::history(&c)?),
        Some(Commands::Recovery { id, yes }) => {
            if let Some(id) = id {
                if !yes {
                    bail!("Inspect the backup, then pass --yes to restore");
                }
                println!("{}", operations::recover(&c, &id)?);
            } else {
                let dir = c.state.join("backups");
                if dir.exists() {
                    for e in fs::read_dir(dir)? {
                        println!("{}", e?.path().display());
                    }
                }
            }
            Ok(())
        }
        Some(Commands::Service { install, start }) => {
            let (p, text) = sync::service_file(&c, &path)?;
            if install {
                println!("{}", sync::install_service(&c, &path, start)?);
            } else {
                println!("{}\n{text}", p.display());
            }
            Ok(())
        }
        Some(Commands::Migrate {
            old,
            destination,
            yes,
        }) => {
            if yes {
                print(&migration::convert(&c, &old, &destination)?)
            } else {
                print(&migration::preview(&old, &destination)?)
            }
        }
        Some(Commands::Source { path: source, yes }) => {
            if !yes {
                println!(
                    "Archive {} and switch to {}. HOME is unchanged. Use --yes to proceed.",
                    c.source.display(),
                    source.display()
                );
                return Ok(());
            }
            println!("{}", ui::switch_source(&mut c, &path, &source)?);
            Ok(())
        }
        Some(Commands::Setting {
            key,
            value,
            scope,
            remove,
            yes,
        }) => {
            if let Some(key) = key {
                if !remove && value.is_none() {
                    bail!("Value required, or use --remove for an override");
                }
                let change = settings::Change {
                    key,
                    scope,
                    value: if remove { None } else { value },
                    identity: settings::identity(&c, scope)?,
                };
                let p = operations::plan(
                    &c,
                    Action::Setting,
                    &[],
                    Some(serde_json::to_string(&change)?),
                )?;
                if yes {
                    let r = operations::execute(&mut c, &path, &p, &p.id)?;
                    print(&r)
                } else {
                    print(&p)
                }
            } else {
                println!("{}", settings::describe(&c)?);
                Ok(())
            }
        }
        Some(Commands::Health) => {
            println!(
                "Git: {}",
                process::run(&c.git, &process::strings(&["--version"]), None, None)?.trim()
            );
            println!(
                "Chezmoi: {}",
                process::run(&c.chezmoi, &process::strings(&["--version"]), None, None)?.trim()
            );
            print(&c)
        }
        Some(Commands::Packages { install }) => {
            for tool in [&c.git, &c.chezmoi] {
                let result = process::run(tool, &process::strings(&["--version"]), None, None);
                println!("{tool}: {}", result.as_deref().unwrap_or("missing"));
            }
            if install {
                let (manager, args) = if cfg!(target_os = "macos") {
                    ("brew", vec!["install", "git", "chezmoi"])
                } else {
                    ("sudo", vec!["pacman", "-S", "--needed", "git", "chezmoi"])
                };
                let status = std::process::Command::new(manager).args(args).status()?;
                if !status.success() {
                    bail!("Dependency installation failed");
                }
            }
            Ok(())
        }
    }
}
fn print<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
