//! Typed errors carrying the CLI exit codes of spec §9.

use std::fmt;

/// Exit codes: 0 ok, 1 error, 2 usage, 3 needs approval (added to pending),
/// 4 vault locked, 5 conflict, 6 refused public remote, 7 stale (state changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Error,
    Usage,
    NeedsApproval,
    VaultLocked,
    Conflict,
    PublicRemote,
    Stale,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Error,
        Kind::Usage,
        Kind::NeedsApproval,
        Kind::VaultLocked,
        Kind::Conflict,
        Kind::PublicRemote,
        Kind::Stale,
    ];

    pub fn code(self) -> u8 {
        match self {
            Kind::Error => 1,
            Kind::Usage => 2,
            Kind::NeedsApproval => 3,
            Kind::VaultLocked => 4,
            Kind::Conflict => 5,
            Kind::PublicRemote => 6,
            Kind::Stale => 7,
        }
    }

    /// Stable identifier used in JSON error objects.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Error => "error",
            Kind::Usage => "usage",
            Kind::NeedsApproval => "needs_approval",
            Kind::VaultLocked => "vault_locked",
            Kind::Conflict => "conflict",
            Kind::PublicRemote => "public_remote",
            Kind::Stale => "stale",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DwError {
    pub kind: Kind,
    pub message: String,
}

impl DwError {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, message)
    }
}

impl fmt::Display for DwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DwError {}

/// The kind and message to report for any error: a [`DwError`] anywhere in the
/// chain decides the kind, everything else is a plain error.
pub fn classify(e: &anyhow::Error) -> (Kind, String) {
    let kind = e
        .chain()
        .find_map(|c| c.downcast_ref::<DwError>())
        .map_or(Kind::Error, |d| d.kind);
    (kind, format!("{e:#}"))
}

/// The JSON error object printed on stdout when `--json` is set.
pub fn to_json(kind: Kind, message: &str) -> serde_json::Value {
    serde_json::json!({"error": {"kind": kind.name(), "code": kind.code(), "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_spec() {
        let codes: Vec<u8> = Kind::ALL.iter().map(|k| k.code()).collect();
        assert_eq!(codes, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn classify_finds_a_typed_error_through_context() {
        use anyhow::Context;
        let e = Err::<(), _>(DwError::new(Kind::Stale, "state changed"))
            .context("while doing x")
            .unwrap_err();
        let (k, m) = classify(&e);
        assert_eq!(k, Kind::Stale);
        assert!(m.contains("while doing x") && m.contains("state changed"));
        assert_eq!(classify(&anyhow::anyhow!("plain")).0, Kind::Error);
    }

    #[test]
    fn json_shape() {
        let v = to_json(Kind::VaultLocked, "locked");
        assert_eq!(v["error"]["code"], 4);
        assert_eq!(v["error"]["kind"], "vault_locked");
        assert_eq!(v["error"]["message"], "locked");
    }
}
