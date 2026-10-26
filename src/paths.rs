//! XDG-aware locations of Dotwarden's machine-local files (spec §3).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// A snapshot of the process environment that is handed to child processes.
pub type Env = BTreeMap<OsString, OsString>;

pub fn process_env() -> Env {
    std::env::vars_os().collect()
}

#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
}

impl Dirs {
    /// Resolve directories from an environment, honouring the XDG variables.
    /// Relative XDG values are ignored, as the XDG specification requires.
    pub fn from_env(env: &Env) -> Result<Self> {
        let get = |key: &str| env.get(&OsString::from(key)).map(PathBuf::from);
        let Some(home) = get("HOME").filter(|p| p.is_absolute()) else {
            bail!("HOME is not set to an absolute path");
        };
        let xdg = |key: &str, fallback: &str| {
            get(key)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        Ok(Self {
            data: xdg("XDG_DATA_HOME", ".local/share"),
            state: xdg("XDG_STATE_HOME", ".local/state"),
            config: xdg("XDG_CONFIG_HOME", ".config"),
            cache: xdg("XDG_CACHE_HOME", ".cache"),
            home,
        })
    }

    /// `$XDG_DATA_HOME/dotwarden`
    pub fn data_dir(&self) -> PathBuf {
        self.data.join("dotwarden")
    }
    /// `$XDG_STATE_HOME/dotwarden`
    pub fn state_dir(&self) -> PathBuf {
        self.state.join("dotwarden")
    }
    /// The git repository.
    pub fn repo(&self) -> PathBuf {
        self.data_dir().join("repo")
    }
    /// Generated chezmoi configuration.
    pub fn chezmoi_config(&self) -> PathBuf {
        self.data_dir().join("chezmoi.toml")
    }
    /// chezmoi persistent state.
    pub fn chezmoi_state(&self) -> PathBuf {
        self.state_dir().join("chezmoistate.boltdb")
    }
    /// chezmoi cache (not listed in the spec; kept under XDG cache).
    pub fn chezmoi_cache(&self) -> PathBuf {
        self.cache.join("dotwarden").join("chezmoi")
    }
    /// Dotwarden's own machine-local directories, wherever XDG puts them.
    pub fn own_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.data_dir(),
            self.state_dir(),
            self.cache.join("dotwarden"),
        ]
    }
    /// The mutation lock file.
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir().join("lock")
    }
    /// Serialises single chezmoi invocations (its state database admits one writer).
    pub fn chezmoi_lock_file(&self) -> PathBuf {
        self.state_dir().join("chezmoi.lock")
    }
    /// The pending list, shared by the CLI and the TUI.
    pub fn pending_file(&self) -> PathBuf {
        self.state_dir().join("pending.json")
    }
    /// This machine's registry name and local confirmations.
    pub fn machine_toml(&self) -> PathBuf {
        self.state_dir().join("machine.toml")
    }
    /// Preferences (an ordinary dotfile).
    pub fn preferences(&self) -> PathBuf {
        self.config.join("dotwarden").join("config.toml")
    }
}

/// Shorten `path` for display by replacing the HOME prefix with `~`.
pub fn tilde(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[test]
    fn defaults_derive_from_home() {
        let d = Dirs::from_env(&env(&[("HOME", "/h")])).unwrap();
        assert_eq!(d.repo(), Path::new("/h/.local/share/dotwarden/repo"));
        assert_eq!(
            d.chezmoi_state(),
            Path::new("/h/.local/state/dotwarden/chezmoistate.boltdb")
        );
        assert_eq!(
            d.preferences(),
            Path::new("/h/.config/dotwarden/config.toml")
        );
    }

    #[test]
    fn xdg_variables_are_respected_and_relative_ones_ignored() {
        let d = Dirs::from_env(&env(&[
            ("HOME", "/h"),
            ("XDG_DATA_HOME", "/x/data"),
            ("XDG_STATE_HOME", "relative/state"),
        ]))
        .unwrap();
        assert_eq!(d.repo(), Path::new("/x/data/dotwarden/repo"));
        assert_eq!(d.state_dir(), Path::new("/h/.local/state/dotwarden"));
    }

    #[test]
    fn missing_home_is_an_error() {
        assert!(Dirs::from_env(&env(&[])).is_err());
    }
}
