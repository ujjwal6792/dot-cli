//! Rendering secrets (spec §7). chezmoi runs the dotwarden binary as a helper
//! (`dw __secret NAME TARGETS`, `dw __file ID`) through its `output` template
//! function. A dotwarden process that runs chezmoi registers a random per-run
//! token (a file named after the token's hash, holding the process id) and
//! hands the token to chezmoi's environment; the helper refuses to print
//! anything without a token of a live registered process, and without an
//! unlock session. The token is a guard against stray invocations, not a
//! boundary against another process of the same user (which can read the
//! unlock session or HOME directly).

use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::error::{DwError, Kind};
use crate::store::Store;
use crate::vault;

const TOKEN_ENV: &str = "DOTWARDEN_RENDER_TOKEN";
pub const HELPER_ENV: &str = "DOTWARDEN_HELPER";
/// Set for renders that must never contain a value (previews as another machine).
pub const MASK_ENV: &str = "DOTWARDEN_MASK";

/// The reference of an encrypted whole file in a stored template.
pub const FILE_MARKER: &str = "{{ includeTemplate \"dotwarden/file\"";

pub fn file_stub(id: &str) -> String {
    format!("{FILE_MARKER} (dict \"id\" \"{id}\") }}}}")
}

pub fn file_id(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(FILE_MARKER)?;
    let rest = rest.strip_prefix(" (dict \"id\" \"")?;
    rest.split('"').next()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run_dir(store: &Store) -> PathBuf {
    store.dirs.state_dir().join("run")
}

struct Token {
    value: String,
    file: PathBuf,
}

static TOKEN: OnceLock<Option<Token>> = OnceLock::new();

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

fn create_token(store: &Store) -> Option<Token> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = run_dir(store);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .ok()?;
    // Registrations of processes that no longer run.
    for e in std::fs::read_dir(&dir).ok()?.flatten() {
        let alive = std::fs::read_to_string(e.path())
            .ok()
            .and_then(|t| t.trim().parse::<i32>().ok())
            .is_some_and(pid_alive);
        if !alive {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let raw: [u8; 32] = rand::random();
    let value = hex(&raw);
    let file = dir.join(hex(&Sha256::digest(value.as_bytes())));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
        .ok()?;
    write!(f, "{}", std::process::id()).ok()?;
    Some(Token { value, file })
}

/// Remove this process's registration (best effort, at exit).
pub fn forget() {
    if let Some(Some(t)) = TOKEN.get() {
        let _ = std::fs::remove_file(&t.file);
    }
}

/// Environment that lets chezmoi's templates call the helper. Only when a
/// vault exists; otherwise there is nothing to render.
pub fn chezmoi_env(store: &Store) -> Vec<(&'static str, String)> {
    if !vault::exists(store) {
        return Vec::new();
    }
    let Some(token) = TOKEN.get_or_init(|| create_token(store)) else {
        return Vec::new();
    };
    let bin = store
        .env()
        .get(OsStr::new("DOTWARDEN_BIN"))
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_default();
    vec![
        (TOKEN_ENV, token.value.clone()),
        (HELPER_ENV, bin.to_string_lossy().into_owned()),
    ]
}

fn verify_token(store: &Store) -> Result<()> {
    let refused = || {
        DwError::new(
            Kind::Error,
            "this helper is run by dotwarden itself; it refuses to print without a valid per-run token",
        )
    };
    let token = store
        .env()
        .get(OsStr::new(TOKEN_ENV))
        .and_then(|v| v.to_str())
        .filter(|v| v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(refused)?;
    let file = run_dir(store).join(hex(&Sha256::digest(token.as_bytes())));
    let alive = std::fs::read_to_string(file)
        .ok()
        .and_then(|t| t.trim().parse::<i32>().ok())
        .is_some_and(pid_alive);
    if alive { Ok(()) } else { Err(refused().into()) }
}

/// `dw __secret NAME TARGETS_JSON`: the value `NAME` has for those targets.
pub fn helper_secret(store: &Store, name: &str, targets_json: &str) -> Result<String> {
    verify_token(store)?;
    let id = vault::session::require(store)?;
    let targets: Vec<String> = serde_json::from_str(targets_json)?;
    crate::secrets::value_for(store, &id, name, &targets)
}

/// `dw __file ID`: the decrypted content of an encrypted file.
pub fn helper_file(store: &Store, id: &str) -> Result<Vec<u8>> {
    verify_token(store)?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        anyhow::bail!("invalid file id");
    }
    let ident = vault::session::require(store)?;
    let bytes = std::fs::read(store.repo().join(vault::rel(&format!("files/{id}.age"))))?;
    vault::decrypt(&ident, &bytes)
}
