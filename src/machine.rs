//! Machine detection and registration (spec §4).

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// One entry of the machine registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    /// `darwin` or `linux`.
    pub platform: String,
    /// `/etc/os-release` `ID`, or `macos` on darwin.
    pub distribution: String,
    /// Distribution families, e.g. `arch-like`.
    pub families: Vec<String>,
    #[serde(default)]
    pub retired: bool,
}

/// Platform facts about the computer dotwarden runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub platform: String,
    pub distribution: String,
    pub families: Vec<String>,
}

impl Detected {
    pub fn into_machine(self, name: &str) -> Machine {
        Machine {
            name: name.to_string(),
            platform: self.platform,
            distribution: self.distribution,
            families: self.families,
            retired: false,
        }
    }
}

/// Distribution IDs that are their own family (they usually have no `ID_LIKE`).
const BASE_FAMILIES: &[&str] = &["arch", "debian", "fedora", "rhel", "suse"];

fn family_target(token: &str) -> String {
    match token {
        "arch" => "arch-like".to_string(),
        other => other.to_string(),
    }
}

/// Detect from an OS name (`std::env::consts::OS`) and the text of os-release.
pub fn detect_from(os: &str, os_release: Option<&str>) -> Result<Detected> {
    match os {
        "macos" => Ok(Detected {
            platform: "darwin".into(),
            distribution: "macos".into(),
            families: vec![],
        }),
        "linux" => {
            let fields = parse_os_release(os_release.unwrap_or(""));
            let get = |k: &str| {
                fields
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.as_str())
            };
            let id = get("ID").unwrap_or("linux").to_ascii_lowercase();
            let mut families: Vec<String> = Vec::new();
            let mut push = |t: &str| {
                let f = family_target(t);
                if !families.contains(&f) {
                    families.push(f);
                }
            };
            for t in get("ID_LIKE").unwrap_or("").split_whitespace() {
                push(&t.to_ascii_lowercase());
            }
            if BASE_FAMILIES.contains(&id.as_str()) {
                push(&id);
            }
            Ok(Detected {
                platform: "linux".into(),
                distribution: id,
                families,
            })
        }
        other => bail!("unsupported platform: {other}"),
    }
}

pub fn parse_os_release(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.trim().split_once('='))
        .map(|(k, v)| {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            (k.trim().to_string(), v.to_string())
        })
        .collect()
}

/// Detect the machine this process runs on.
pub fn detect() -> Result<Detected> {
    let text = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|p| std::fs::read_to_string(Path::new(p)).ok());
    detect_from(std::env::consts::OS, text.as_deref())
}

/// Machine names are `[a-z0-9-]`, non-empty, and neither start nor end with `-`.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    if !ok {
        bail!(
            "invalid machine name {name:?}: use 1-63 characters from a-z, 0-9 and '-', \
             not starting or ending with '-'"
        );
    }
    Ok(())
}

/// A name suggestion derived from the hostname (never required to match it).
pub fn suggest_name(hostname: &str) -> String {
    let s: String = hostname
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "machine".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CACHYOS: &str = "NAME=\"CachyOS Linux\"\nID=cachyos\nID_LIKE=arch\n";
    const ARCH: &str = "NAME=\"Arch Linux\"\nID=arch\n";
    const UBUNTU: &str = "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\nVERSION_ID=\"24.04\"\n";
    const DEBIAN: &str = "ID=debian\n";

    #[test]
    fn detects_cachyos_as_arch_like() {
        let d = detect_from("linux", Some(CACHYOS)).unwrap();
        assert_eq!(d.platform, "linux");
        assert_eq!(d.distribution, "cachyos");
        assert_eq!(d.families, ["arch-like"]);
    }

    #[test]
    fn arch_is_its_own_family() {
        let d = detect_from("linux", Some(ARCH)).unwrap();
        assert_eq!(
            (d.distribution.as_str(), d.families),
            ("arch", vec!["arch-like".to_string()])
        );
    }

    #[test]
    fn ubuntu_belongs_to_debian() {
        let d = detect_from("linux", Some(UBUNTU)).unwrap();
        assert_eq!(d.distribution, "ubuntu");
        assert_eq!(d.families, ["debian"]);
        assert_eq!(
            detect_from("linux", Some(DEBIAN)).unwrap().families,
            ["debian"]
        );
    }

    #[test]
    fn macos_is_darwin() {
        let d = detect_from("macos", None).unwrap();
        assert_eq!(
            (d.platform.as_str(), d.distribution.as_str()),
            ("darwin", "macos")
        );
        assert!(d.families.is_empty());
    }

    #[test]
    fn unsupported_and_missing_os_release() {
        assert!(detect_from("windows", None).is_err());
        assert_eq!(detect_from("linux", None).unwrap().distribution, "linux");
    }

    #[test]
    fn names() {
        for ok in ["mac", "work-laptop2", "a"] {
            validate_name(ok).unwrap();
        }
        for bad in ["", "Mac", "-a", "a-", "a_b", "a b", "é"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        assert_eq!(
            suggest_name("Ujjwals-MacBook-Pro.local"),
            "ujjwals-macbook-pro"
        );
        assert_eq!(suggest_name("..."), "machine");
    }
}
