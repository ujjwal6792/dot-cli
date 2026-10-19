use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip)]
    pub runtime_config: Option<PathBuf>,
    pub home: PathBuf,
    pub source: PathBuf,
    pub state: PathBuf,
    pub chezmoi_config: Option<PathBuf>,
    pub chezmoi: String,
    pub git: String,
    pub machine: String,
    pub installation: String,
    pub enabled: BTreeSet<String>,
    pub sync_enabled: bool,
    pub trusted: bool,
    pub reporting: bool,
    pub interval_seconds: u64,
    pub retention_days: u32,
}
impl Default for Config {
    fn default() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into()));
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        Self {
            runtime_config: None,
            source: data.join("chezmoi"),
            state: state.join("dot"),
            home,
            chezmoi_config: None,
            chezmoi: "chezmoi".into(),
            git: "git".into(),
            machine: std::env::var("HOSTNAME")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    std::process::Command::new("hostname")
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                })
                .unwrap_or_else(|| "my-machine".into()),
            installation: uuid::Uuid::new_v4().to_string(),
            enabled: BTreeSet::new(),
            sync_enabled: false,
            trusted: false,
            reporting: false,
            interval_seconds: 300,
            retention_days: 30,
        }
    }
}
impl Config {
    pub fn default_path() -> PathBuf {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("dot/config.toml")
    }
    pub fn load(path: &Path) -> Result<Self> {
        let mut c: Self = if path.exists() {
            toml::from_str(&fs::read_to_string(path)?)
                .context("Invalid dot config; original was left untouched")?
        } else {
            Self::default()
        };
        c.runtime_config = Some(path.to_path_buf());
        Ok(c)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        atomic_write(path, toml::to_string_pretty(self)?.as_bytes())
    }
    pub fn resolve_tools(&mut self) {
        for tool in [&mut self.git, &mut self.chezmoi] {
            if Path::new(tool).is_absolute() {
                continue;
            }
            if let Some(paths) = std::env::var_os("PATH")
                && let Some(p) = std::env::split_paths(&paths)
                    .map(|dir| dir.join(&*tool))
                    .find(|p| p.is_file())
            {
                *tool = p.to_string_lossy().into_owned();
            }
        }
    }
    pub fn validate(&self) -> Result<()> {
        for p in [&self.home, &self.source, &self.state] {
            if !p.is_absolute() {
                bail!("Path must be absolute: {}", p.display());
            }
        }
        if self.source == self.home
            || self.state == self.home
            || self.state.starts_with(&self.source)
        {
            bail!("Source, HOME, and state must be separate; state cannot be inside source");
        }
        if self.interval_seconds < 30 {
            bail!("Sync interval must be at least 30 seconds");
        }
        Ok(())
    }
    pub fn relative(&self, path: &str) -> Result<String> {
        let p = Path::new(path);
        let rel = if p.is_absolute() {
            p.strip_prefix(&self.home)
                .context("Target must be inside configured HOME")?
        } else {
            p
        };
        clean_relative(rel)
    }
    pub fn target(&self, path: &str) -> Result<PathBuf> {
        let rel = self.relative(path)?;
        // A final symlink can be a managed object; a symlinked parent could escape HOME.
        let mut parent = self.home.clone();
        let parts: Vec<_> = Path::new(&rel).components().collect();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            parent.push(part.as_os_str());
            if fs::symlink_metadata(&parent).is_ok_and(|m| m.file_type().is_symlink()) {
                bail!(
                    "Symlink parent requires manual chezmoi handling: {}",
                    parent.display()
                );
            }
        }
        Ok(self.home.join(rel))
    }
}
pub fn clean_relative(path: &Path) -> Result<String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!(
            "Expected an exact relative path without . or ..: {}",
            path.display()
        );
    }
    path.to_str()
        .map(str::to_string)
        .context("Non-UTF-8 paths require direct chezmoi/Git")
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn native_config_paths(c: &Config) -> Vec<PathBuf> {
    if let Some(path) = &c.chezmoi_config {
        return vec![path.clone()];
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| c.home.clone());
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("chezmoi");
    [
        "chezmoi.toml",
        "chezmoi.yaml",
        "chezmoi.yml",
        "chezmoi.json",
    ]
    .iter()
    .map(|name| root.join(name))
    .collect()
}
