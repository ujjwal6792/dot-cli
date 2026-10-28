//! The isolated store (spec §3, ADR 0002): every chezmoi and git invocation is
//! built here, so no caller can accidentally read or modify a user's own chezmoi
//! installation or an unrelated git environment.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};

use crate::exec::{self, CancelToken, Output};
use crate::paths::{Dirs, Env, process_env};
use crate::toolchain::{self, Toolchain};
use crate::{fsutil, lock};

/// Format version recorded in `dotwarden.toml`.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Store {
    pub dirs: Dirs,
    pub tools: Toolchain,
    env: Env,
    identity: Arc<OnceLock<Vec<(String, String)>>>,
}

impl Store {
    pub fn new(dirs: Dirs, tools: Toolchain, env: Env) -> Self {
        Self {
            dirs,
            tools,
            env,
            identity: Arc::default(),
        }
    }

    /// Build from the process environment and the bundled tools.
    pub fn from_process() -> Result<Self> {
        let env = process_env();
        let dirs = Dirs::from_env(&env)?;
        let tools = Toolchain::resolve(&env)?;
        Ok(Self::new(dirs, tools, env))
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn repo(&self) -> PathBuf {
        self.dirs.repo()
    }

    /// Whether a Dotwarden repository exists in the store.
    pub fn is_initialised(&self) -> bool {
        let repo = self.repo();
        repo.join(".chezmoiroot").is_file() && repo.join("dotwarden.toml").is_file()
    }

    /// Create the state and cache directories chezmoi expects to exist.
    pub fn ensure_dirs(&self) -> Result<()> {
        for d in [
            self.dirs.data_dir(),
            self.dirs.state_dir(),
            self.dirs.chezmoi_cache(),
        ] {
            fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(())
    }

    // ---- chezmoi ------------------------------------------------------

    /// A chezmoi command with the explicit isolation flags and a scrubbed
    /// environment: no `CHEZMOI_*`, the bundled git first on `PATH`, and the
    /// same git isolation as [`Store::git_command`] for the git chezmoi runs
    /// itself. Callers append the subcommand and its arguments.
    pub fn chezmoi_command(&self) -> Command {
        self.chezmoi_command_with(
            &self.dirs.chezmoi_config(),
            &self.dirs.chezmoi_state(),
            &self.dirs.chezmoi_cache(),
        )
    }

    /// Like [`Store::chezmoi_command`] with another config, state and cache
    /// (preview as another machine uses throwaway ones).
    pub fn chezmoi_command_with(&self, config: &Path, state: &Path, cache: &Path) -> Command {
        let mut cmd = Command::new(self.tools.chezmoi());
        cmd.env_clear();
        for (k, v) in &self.env {
            let key = k.to_str().unwrap_or_default();
            if !toolchain::is_scrubbed_for_chezmoi(key) && !toolchain::is_scrubbed_for_git(key) {
                cmd.env(k, v);
            }
        }
        self.apply_git_env(&mut cmd);
        let mut path = vec![self.tools.git_root().join("bin")];
        if let Some(existing) = self.env.get(OsStr::new("PATH")) {
            path.extend(std::env::split_paths(existing));
        }
        if let Ok(joined) = std::env::join_paths(path) {
            cmd.env("PATH", joined);
        }
        cmd.arg("--config")
            .arg(config)
            .arg("--source")
            .arg(self.repo())
            .arg("--persistent-state")
            .arg(state)
            .arg("--cache")
            .arg(cache)
            .arg("--destination")
            .arg(&self.dirs.home)
            .arg("--no-tty");
        cmd
    }

    /// Run chezmoi to completion without writing anything of dotwarden's own:
    /// it waits for the chezmoi lock only if some other command created it.
    pub fn chezmoi<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Output> {
        let _guard = lock::acquire_existing(
            &self.dirs.chezmoi_lock_file(),
            lock::wait_from_env(&self.env),
        )?;
        let mut cmd = self.chezmoi_command();
        cmd.args(args);
        exec::run_ok(cmd, None)
    }

    /// Run chezmoi for planning and mutations. Each invocation holds a short lock
    /// of its own, so planning (which only previews) can run outside the
    /// mutation lock without two chezmoi processes fighting over its state
    /// database.
    pub fn chezmoi_locked<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Output> {
        let _guard = self.chezmoi_guard()?;
        let mut cmd = self.chezmoi_command();
        cmd.args(args);
        exec::run_ok(cmd, None)
    }

    pub fn chezmoi_guard(&self) -> Result<lock::Lock> {
        lock::acquire(
            &self.dirs.chezmoi_lock_file(),
            lock::wait_from_env(&self.env),
        )
    }

    /// The generated chezmoi configuration for `machine_name`. The registry
    /// name is the only machine fact chezmoi needs; everything else is looked
    /// up in the registry by the fixed templates.
    pub fn chezmoi_config_body(machine_name: &str) -> Result<String> {
        let mut name = toml::Table::new();
        name.insert("name".into(), machine_name.into());
        let mut data = toml::Table::new();
        data.insert("dotwardenMachine".into(), name.into());
        let mut git = toml::Table::new();
        git.insert("autoCommit".into(), false.into());
        git.insert("autoPush".into(), false.into());
        let mut root = toml::Table::new();
        root.insert("data".into(), data.into());
        root.insert("git".into(), git.into());
        Ok(format!(
            "# Generated by dotwarden; regenerated on demand, never hand-edited.\n{}",
            toml::to_string(&root)?
        ))
    }

    /// Write the generated chezmoi configuration (atomically, only if it differs).
    pub fn write_chezmoi_config(&self, machine_name: &str) -> Result<()> {
        self.ensure_dirs()?;
        let body = Self::chezmoi_config_body(machine_name)?;
        let path = self.dirs.chezmoi_config();
        if fs::read_to_string(&path).ok().as_deref() != Some(body.as_str()) {
            fsutil::write_atomic(&path, body)?;
        }
        Ok(())
    }

    // ---- git ----------------------------------------------------------

    /// Settings forwarded from the user's global git configuration: identity
    /// and signing only. Everything else in `~/.gitconfig` (hooks, url
    /// rewrites, fsmonitor, aliases, ...) is deliberately not applied.
    pub fn git_identity(&self) -> &[(String, String)] {
        self.identity.get_or_init(|| self.read_git_identity())
    }

    pub fn git_identity_value(&self, key: &str) -> Option<&str> {
        self.git_identity()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn read_git_identity(&self) -> Vec<(String, String)> {
        let mut cmd = Command::new(self.tools.git_bin());
        cmd.env_clear();
        for (k, v) in &self.env {
            let key = k.to_str().unwrap_or_default();
            // The user's own GIT_CONFIG_GLOBAL names their global config.
            if !toolchain::is_scrubbed_for_git(key) || key == "GIT_CONFIG_GLOBAL" {
                cmd.env(k, v);
            }
        }
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_EXEC_PATH", self.tools.git_exec_path())
            .current_dir(&self.dirs.home)
            .args(["config", "--global", "--null", "--get-regexp", "."]);
        let Ok(out) = exec::run(cmd, None) else {
            return Vec::new();
        };
        if !out.success() {
            return Vec::new();
        }
        let mut found: Vec<(String, String)> = Vec::new();
        for entry in out.stdout.split('\0').filter(|e| !e.is_empty()) {
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            if toolchain::is_forwarded_git_key(key) {
                found.retain(|(k, _)| k != key);
                found.push((key.to_string(), value.to_string()));
            }
        }
        found
    }

    /// Environment that isolates git: no system or global configuration, the
    /// portable git's own paths, and the forwarded identity as `GIT_CONFIG_*`
    /// pairs (which also reach git processes started by chezmoi).
    fn apply_git_env(&self, cmd: &mut Command) {
        cmd.env("GIT_CONFIG_NOSYSTEM", "1");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
        for (k, v) in self.tools.git_env() {
            cmd.env(k, v);
        }
        let identity = self.git_identity();
        cmd.env("GIT_CONFIG_COUNT", identity.len().to_string());
        for (i, (k, v)) in identity.iter().enumerate() {
            cmd.env(format!("GIT_CONFIG_KEY_{i}"), k);
            cmd.env(format!("GIT_CONFIG_VALUE_{i}"), v);
        }
    }

    /// A bundled-git command running in `cwd`. `GIT_*` variables that could
    /// redirect git are removed; `GIT_SSH_COMMAND`, `SSH_AUTH_SOCK` and the like
    /// pass through so SSH remotes keep working.
    pub fn git_command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(self.tools.git_bin());
        cmd.env_clear();
        for (k, v) in &self.env {
            if !k.to_str().is_some_and(toolchain::is_scrubbed_for_git) {
                cmd.env(k, v);
            }
        }
        self.apply_git_env(&mut cmd);
        cmd.current_dir(cwd);
        cmd
    }

    /// Run git inside the repository.
    pub fn git<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Output> {
        self.git_cancellable(args, None)
    }

    pub fn git_cancellable<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        cancel: Option<&CancelToken>,
    ) -> Result<Output> {
        let mut cmd = self.git_command(&self.repo());
        cmd.args(args);
        exec::run_ok(cmd, cancel)
    }

    // ---- machine.toml -------------------------------------------------

    /// This machine's registry name from `machine.toml`.
    pub fn local_machine_name(&self) -> Result<Option<String>> {
        let path = self.dirs.machine_toml();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let t: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(t.get("name").and_then(|v| v.as_str()).map(str::to_string))
    }

    pub fn write_local_machine(&self, name: &str) -> Result<()> {
        self.ensure_dirs()?;
        // Keep what else lives in machine.toml (confirmed remotes).
        let mut t = fs::read_to_string(self.dirs.machine_toml())
            .ok()
            .and_then(|s| toml::from_str::<toml::Table>(&s).ok())
            .unwrap_or_default();
        t.insert("name".into(), name.into());
        fsutil::write_atomic(&self.dirs.machine_toml(), toml::to_string(&t)?)
    }

    /// The mutation lock for this store (spec §9).
    pub fn lock(&self) -> Result<lock::Lock> {
        lock::acquire(&self.dirs.lock_file(), lock::wait_from_env(&self.env))
    }
}
