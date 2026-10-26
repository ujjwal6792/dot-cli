//! Pinned tool versions, read from the single pins file `toolchain/pins.env`.

const PINS: &str = include_str!("../toolchain/pins.env");

/// Look up a `KEY=VALUE` entry in the pins file.
pub fn get(key: &str) -> Option<&'static str> {
    PINS.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

pub fn chezmoi_version() -> &'static str {
    get("CHEZMOI_VERSION").expect("CHEZMOI_VERSION pinned")
}
pub fn git_version() -> &'static str {
    get("GIT_VERSION").expect("GIT_VERSION pinned")
}
pub fn delta_version() -> &'static str {
    get("DELTA_VERSION").expect("DELTA_VERSION pinned")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_pinned() {
        assert_eq!(chezmoi_version(), "2.73.3");
        assert_eq!(git_version(), "2.56.0");
        assert_eq!(delta_version(), "0.20.1");
    }

    #[test]
    fn every_pinned_archive_has_a_checksum() {
        for line in PINS.lines().filter_map(|l| l.split_once('=')) {
            if let Some(suffix) = line.0.strip_prefix("CHEZMOI_FILE_") {
                let sum = get(&format!("CHEZMOI_SHA256_{suffix}")).unwrap();
                assert_eq!(sum.len(), 64);
            }
        }
    }
}
