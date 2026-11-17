//! Preferences (spec §12): `~/.config/dotwarden/config.toml`, read only here.
//! A missing file, an unparsable file or an invalid value gives the default.

use crate::store::Store;

/// Default of `backup_warn_days`.
pub const BACKUP_WARN_DAYS: u64 = 7;

#[derive(Debug, Default)]
pub struct Prefs {
    table: toml::Table,
}

impl Prefs {
    pub fn load(store: &Store) -> Self {
        let table = std::fs::read_to_string(store.dirs.preferences())
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
            .unwrap_or_default();
        Self { table }
    }

    /// The raw value at a dotted key path (`["vault", "unlock_minutes"]`).
    pub fn get(&self, path: &[&str]) -> Option<&toml::Value> {
        let (first, rest) = path.split_first()?;
        rest.iter()
            .try_fold(self.table.get(*first)?, |v, k| v.get(*k))
    }

    /// A non-negative integer at `path`.
    pub fn uint(&self, path: &[&str]) -> Option<u64> {
        self.get(path)?
            .as_integer()
            .and_then(|n| u64::try_from(n).ok())
    }

    /// How many days unbacked changes may wait before status warns.
    pub fn backup_warn_days(&self) -> u64 {
        self.uint(&["backup_warn_days"]).unwrap_or(BACKUP_WARN_DAYS)
    }
}
