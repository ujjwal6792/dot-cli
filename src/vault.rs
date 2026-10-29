//! The vault (spec §7, ADR 0004): one age X25519 identity, wrapped by a
//! passphrase and by a recovery code, plus the unlock session that holds the
//! decrypted identity for a limited time.
//!
//! Secret input comes only from an interactive terminal. Debug builds (never
//! release builds) additionally honour the `DOTWARDEN_TEST_*` variables listed
//! in [`Prompt::hook`] so that tests can run without a terminal.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use age::secrecy::{ExposeSecret, SecretString};
use age::x25519;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DwError, Kind};
use crate::store::Store;
use crate::track::refuse;

pub const ROOT: &str = "home/.dotwarden/vault";
const DEFAULT_MINUTES: u64 = 15;
const WORK_FACTOR: u8 = 18;

pub fn locked() -> anyhow::Error {
    DwError::new(
        Kind::VaultLocked,
        "the vault is locked; run `dw vault unlock` in a terminal",
    )
    .into()
}

pub fn rel(name: &str) -> String {
    format!("{ROOT}/{name}")
}

pub fn exists(store: &Store) -> bool {
    store.repo().join(rel("recipient.txt")).is_file()
}

pub fn recipient(store: &Store) -> Result<x25519::Recipient> {
    let text = std::fs::read_to_string(store.repo().join(rel("recipient.txt")))
        .map_err(|_| refuse("there is no vault yet; run `dw vault init`".into()))?;
    text.trim()
        .parse::<x25519::Recipient>()
        .map_err(|e| anyhow!("recipient.txt is invalid: {e}"))
}

// ---- secret input ------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Prompt {
    Passphrase,
    NewPassphrase,
    RecoveryCode,
    SecretValue,
}

impl Prompt {
    /// Test-only variable that stands in for the terminal. It exists only when
    /// `debug_assertions` is on, so it cannot be enabled in a release build.
    #[cfg(debug_assertions)]
    fn hook(self) -> &'static str {
        match self {
            Prompt::Passphrase => "DOTWARDEN_TEST_PASSPHRASE",
            Prompt::NewPassphrase => "DOTWARDEN_TEST_NEW_PASSPHRASE",
            Prompt::RecoveryCode => "DOTWARDEN_TEST_RECOVERY_CODE",
            Prompt::SecretValue => "DOTWARDEN_TEST_SECRET_VALUE",
        }
    }
}

/// Read a secret from the terminal only: never from arguments, the environment
/// or a pipe. An agent (`DOTWARDEN_AGENT`) is refused even on a terminal.
pub fn read_secret(store: &Store, prompt: Prompt, label: &str) -> Result<String> {
    if crate::changes::agent(store).is_some() {
        return Err(DwError::usage("agents cannot enter secrets; a human must run this").into());
    }
    #[cfg(debug_assertions)]
    if let Some(v) = store.env().get(std::ffi::OsStr::new(prompt.hook())) {
        return Ok(v.to_string_lossy().into_owned());
    }
    #[cfg(not(debug_assertions))]
    let _ = prompt;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(DwError::usage(format!(
            "{label} is read only from an interactive terminal (never from arguments, the environment or a pipe)"
        ))
        .into());
    }
    rpassword::prompt_password(format!("{label}: ")).context("reading from the terminal")
}

/// Ask twice and require both to match.
pub fn read_new_passphrase(store: &Store, label: &str) -> Result<String> {
    let a = read_secret(store, Prompt::NewPassphrase, label)?;
    if a.chars().count() < 8 {
        return Err(refuse(
            "the passphrase must be at least 8 characters".into(),
        ));
    }
    let b = read_secret(store, Prompt::NewPassphrase, "Repeat the passphrase")?;
    if a != b {
        return Err(refuse("the passphrases differ".into()));
    }
    Ok(a)
}

// ---- age helpers -------------------------------------------------------

fn work_factor(store: &Store) -> u8 {
    #[cfg(debug_assertions)]
    if let Some(v) = store
        .env()
        .get(std::ffi::OsStr::new("DOTWARDEN_TEST_WORK_FACTOR"))
        .and_then(|v| v.to_str()?.parse().ok())
    {
        return v;
    }
    let _ = store;
    WORK_FACTOR
}

fn wrap(store: &Store, pass: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let mut r = age::scrypt::Recipient::new(SecretString::from(pass.to_string()));
    r.set_work_factor(work_factor(store));
    age::encrypt(&r, plain).map_err(|e| anyhow!("encrypting: {e}"))
}

fn unwrap(pass: &str, bytes: &[u8], what: &str) -> Result<Vec<u8>> {
    let mut id = age::scrypt::Identity::new(SecretString::from(pass.to_string()));
    id.set_max_work_factor(24);
    age::decrypt(&id, bytes).map_err(|_| refuse(format!("wrong {what}")))
}

/// Encrypt to the vault recipient: needs no unlock.
pub fn encrypt(store: &Store, plain: &[u8]) -> Result<Vec<u8>> {
    let r = recipient(store)?;
    age::encrypt(&r, plain).map_err(|e| anyhow!("encrypting: {e}"))
}

/// Decrypt with an unlocked identity.
pub fn decrypt(id: &x25519::Identity, bytes: &[u8]) -> Result<Vec<u8>> {
    age::decrypt(id, bytes).map_err(|e| anyhow!("decrypting: {e}"))
}

fn parse_identity(text: &[u8]) -> Result<x25519::Identity> {
    std::str::from_utf8(text)
        .ok()
        .and_then(|t| t.trim().parse::<x25519::Identity>().ok())
        .ok_or_else(|| anyhow!("the vault identity is invalid"))
}

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn recovery_code() -> String {
    let bytes: [u8; 20] = rand::random();
    let mut bits = 0u32;
    let mut have = 0;
    let mut out = String::new();
    for b in bytes {
        bits = (bits << 8) | u32::from(b);
        have += 8;
        while have >= 5 {
            have -= 5;
            out.push(B32[((bits >> have) & 31) as usize] as char);
        }
    }
    out.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

fn normalise_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

// ---- repository files --------------------------------------------------

/// Write files below the repository and stage them (the repository stays equal
/// to its index).
pub fn write_staged(store: &Store, files: &[(String, Vec<u8>)], remove: &[String]) -> Result<()> {
    let _lock = store.lock()?;
    crate::track::ensure_repo_clean(store)?;
    let repo = store.repo();
    for (rel, bytes) in files {
        let p = repo.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::fsutil::write_atomic(&p, bytes)?;
    }
    for rel in remove {
        let p = repo.join(rel);
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    let all: Vec<String> = files
        .iter()
        .map(|(r, _)| r.clone())
        .chain(remove.iter().cloned())
        .collect();
    crate::track::stage(store, &all)
}

fn read_wrapped(store: &Store, file: &str) -> Result<Vec<u8>> {
    std::fs::read(store.repo().join(rel(file)))
        .map_err(|_| refuse("there is no vault yet; run `dw vault init`".into()))
}

/// Create the vault; returns the recovery code (shown once by the caller).
pub fn init(store: &Store, pass: &str) -> Result<String> {
    if exists(store) {
        return Err(refuse("the vault already exists".into()));
    }
    let id = x25519::Identity::generate();
    let secret = id.to_string();
    let code = recovery_code();
    let files = vec![
        (
            rel("recipient.txt"),
            format!("{}\n", id.to_public()).into_bytes(),
        ),
        (
            rel("identity.age"),
            wrap(store, pass, secret.expose_secret().as_bytes())?,
        ),
        (
            rel("recovery.age"),
            wrap(
                store,
                &normalise_code(&code),
                secret.expose_secret().as_bytes(),
            )?,
        ),
    ];
    write_staged(store, &files, &[])?;
    session::start(store, &id)?;
    Ok(code)
}

pub fn unlock(store: &Store, pass: &str) -> Result<()> {
    let id = parse_identity(&unwrap(
        pass,
        &read_wrapped(store, "identity.age")?,
        "passphrase",
    )?)?;
    session::start(store, &id)
}

pub fn passwd(store: &Store, old: &str, new: &str) -> Result<()> {
    let plain = unwrap(old, &read_wrapped(store, "identity.age")?, "passphrase")?;
    write_staged(
        store,
        &[(rel("identity.age"), wrap(store, new, &plain)?)],
        &[],
    )
}

pub fn recover(store: &Store, code: &str, new: &str) -> Result<()> {
    let plain = unwrap(
        &normalise_code(code),
        &read_wrapped(store, "recovery.age")?,
        "recovery code",
    )?;
    let id = parse_identity(&plain)?;
    write_staged(
        store,
        &[(rel("identity.age"), wrap(store, new, &plain)?)],
        &[],
    )?;
    session::start(store, &id)
}

// ---- unlock session ----------------------------------------------------

pub mod session {
    use super::*;
    use std::process::{Command, Stdio};

    #[derive(Serialize, Deserialize)]
    struct Payload {
        identity: String,
        expires: u64,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct Status {
        pub unlocked: bool,
        pub expires_in_seconds: Option<u64>,
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    trait Backend {
        fn put(&self, key: &str, payload: &str) -> Result<()>;
        fn get(&self, key: &str) -> Result<Option<String>>;
        fn delete(&self, key: &str) -> Result<()>;
    }

    /// A 0600 file in a private directory (`$XDG_RUNTIME_DIR`, else a private
    /// directory in the temporary directory).
    pub(super) struct FileBackend(pub PathBuf);

    pub(super) fn runtime_dir(store: &Store) -> PathBuf {
        match store
            .env()
            .get(std::ffi::OsStr::new("XDG_RUNTIME_DIR"))
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            Some(p) => p.join("dotwarden"),
            // SAFETY: geteuid has no preconditions.
            None => std::env::temp_dir().join(format!("dotwarden-{}", unsafe { libc::geteuid() })),
        }
    }

    impl FileBackend {
        fn path(&self, key: &str) -> PathBuf {
            self.0.join(format!("session-{key}"))
        }
        fn private_dir(&self) -> Result<()> {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.0)?;
            let m = std::fs::metadata(&self.0)?;
            // SAFETY: geteuid has no preconditions.
            if m.uid() != unsafe { libc::geteuid() } {
                return Err(anyhow!("{} is not owned by you", self.0.display()));
            }
            if m.mode() & 0o077 != 0 {
                std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
    }

    impl Backend for FileBackend {
        fn put(&self, key: &str, payload: &str) -> Result<()> {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            self.private_dir()?;
            let p = self.path(key);
            let _ = std::fs::remove_file(&p);
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&p)?;
            f.write_all(payload.as_bytes())?;
            Ok(())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(std::fs::read_to_string(self.path(key)).ok())
        }
        fn delete(&self, key: &str) -> Result<()> {
            let _ = std::fs::remove_file(self.path(key));
            Ok(())
        }
    }

    const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// Run an OS credential tool without ever blocking on it: stdin is `input`
    /// (or null), the child is killed after [`TOOL_TIMEOUT`], and any failure is
    /// an error (the caller falls back to the file store). Returns stdout.
    fn run_tool(mut cmd: Command, input: Option<&str>) -> Result<String> {
        use std::io::{Read, Write};
        cmd.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        let mut child = cmd.spawn().context("starting the credential tool")?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin.write_all(text.as_bytes())?;
        }
        let start = std::time::Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s;
            }
            if start.elapsed() >= TOOL_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("the credential tool timed out"));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let mut out = String::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_string(&mut out);
        }
        if status.success() {
            Ok(out)
        } else {
            Err(anyhow!("the credential tool failed"))
        }
    }

    fn hex_of(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(s: &str) -> Option<String> {
        let s = s.trim();
        let bytes: Option<Vec<u8>> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect();
        String::from_utf8(bytes?).ok()
    }

    /// macOS Keychain through the `security` tool, always with an explicit
    /// keychain file (so it never searches for or creates one, which would
    /// raise a GUI dialog) and never blocking. The payload travels hex-encoded
    /// on the stdin of `security -i`, not on a command line.
    struct Keychain(PathBuf);

    impl Keychain {
        /// A usable login keychain of this HOME, checked without any prompt.
        fn open(home: &Path) -> Result<Self> {
            let path = home.join("Library/Keychains/login.keychain-db");
            if !path.is_file() {
                return Err(anyhow!("no login keychain in {}", home.display()));
            }
            let mut cmd = Command::new("security");
            cmd.arg("show-keychain-info").arg(&path);
            run_tool(cmd, None).context("the login keychain is not usable")?;
            Ok(Self(path))
        }
        fn line(&self, verb: &str, key: &str, tail: &str) -> String {
            format!(
                "{verb} -a {key} -s dotwarden-session {tail} \"{}\"\n",
                self.0.display()
            )
        }
        fn run(&self, line: &str) -> Result<String> {
            let mut cmd = Command::new("security");
            cmd.arg("-i");
            run_tool(cmd, Some(line))
        }
    }

    impl Backend for Keychain {
        fn put(&self, key: &str, payload: &str) -> Result<()> {
            let tail = format!("-U -w {}", hex_of(payload));
            self.run(&self.line("add-generic-password", key, &tail))
                .map(|_| ())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self
                .run(&self.line("find-generic-password", key, "-w"))
                .ok()
                .and_then(|o| unhex(&o)))
        }
        fn delete(&self, key: &str) -> Result<()> {
            let _ = self.run(&self.line("delete-generic-password", key, ""));
            Ok(())
        }
    }

    /// Linux Secret Service through `secret-tool` (payload on stdin, bounded).
    struct SecretService;

    impl Backend for SecretService {
        fn put(&self, key: &str, payload: &str) -> Result<()> {
            let mut cmd = Command::new("secret-tool");
            cmd.args([
                "store",
                "--label=dotwarden session",
                "service",
                "dotwarden",
                "key",
                key,
            ]);
            run_tool(cmd, Some(payload)).map(|_| ())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            let mut cmd = Command::new("secret-tool");
            cmd.args(["lookup", "service", "dotwarden", "key", key]);
            Ok(run_tool(cmd, None).ok().filter(|o| !o.is_empty()))
        }
        fn delete(&self, key: &str) -> Result<()> {
            let mut cmd = Command::new("secret-tool");
            cmd.args(["clear", "service", "dotwarden", "key", key]);
            let _ = run_tool(cmd, None);
            Ok(())
        }
    }

    /// The OS store, falling back to the file when it is unavailable.
    struct Auto {
        primary: Option<Box<dyn Backend>>,
        /// Why there is no OS store (shown once when a session starts).
        unavailable: Option<String>,
        file: FileBackend,
    }

    fn notice(why: &str) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            eprintln!("dotwarden: OS credential store unavailable ({why}); using a private 0600 file instead");
        });
    }

    impl Backend for Auto {
        fn put(&self, key: &str, payload: &str) -> Result<()> {
            match (&self.primary, &self.unavailable) {
                (Some(p), _) => match p.put(key, payload) {
                    Ok(()) => return Ok(()),
                    Err(e) => notice(&format!("{e:#}")),
                },
                (None, Some(why)) => notice(why),
                _ => {}
            }
            self.file.put(key, payload)
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            if let Some(Some(v)) = self.primary.as_ref().map(|p| p.get(key).ok().flatten()) {
                return Ok(Some(v));
            }
            self.file.get(key)
        }
        fn delete(&self, key: &str) -> Result<()> {
            if let Some(p) = &self.primary {
                p.delete(key)?;
            }
            self.file.delete(key)
        }
    }

    /// `DOTWARDEN_SESSION_BACKEND=file` selects the file store (tests use it so
    /// that no real keychain is touched).
    fn backend(store: &Store) -> Auto {
        let file = FileBackend(runtime_dir(store));
        let forced = store
            .env()
            .get(std::ffi::OsStr::new("DOTWARDEN_SESSION_BACKEND"))
            .is_some_and(|v| v == "file");
        // A tool that failed or hung once is not tried again in this process.
        static FAILED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let found: Result<Box<dyn Backend>> = if forced {
            Err(anyhow!("forced"))
        } else if let Some(why) = FAILED.get() {
            Err(anyhow!("{why}"))
        } else if cfg!(target_os = "macos") {
            Keychain::open(&store.dirs.home).map(|k| Box::new(k) as Box<dyn Backend>)
        } else {
            let mut cmd = Command::new("secret-tool");
            cmd.arg("--version");
            run_tool(cmd, None).map(|_| Box::new(SecretService) as Box<dyn Backend>)
        };
        if let (Err(e), false) = (&found, forced) {
            let _ = FAILED.set(format!("{e:#}"));
        }
        match found {
            Ok(p) => Auto {
                primary: Some(p),
                unavailable: None,
                file,
            },
            Err(e) => Auto {
                primary: None,
                unavailable: (!forced).then(|| format!("{e:#}")),
                file,
            },
        }
    }

    fn key(store: &Store) -> String {
        let d = Sha256::digest(store.repo().to_string_lossy().as_bytes());
        d.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    fn minutes(store: &Store) -> u64 {
        std::fs::read_to_string(store.dirs.preferences())
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
            .and_then(|t| t.get("vault")?.get("unlock_minutes")?.as_integer())
            .filter(|m| *m > 0)
            .map_or(DEFAULT_MINUTES, |m| m as u64)
    }

    pub fn start(store: &Store, id: &x25519::Identity) -> Result<()> {
        let payload = serde_json::to_string(&Payload {
            identity: id.to_string().expose_secret().to_string(),
            expires: now() + minutes(store) * 60,
        })?;
        backend(store).put(&key(store), &payload)
    }

    pub fn end(store: &Store) -> Result<()> {
        backend(store).delete(&key(store))
    }

    fn load(store: &Store) -> Option<Payload> {
        let b = backend(store);
        let p: Payload = serde_json::from_str(&b.get(&key(store)).ok()??).ok()?;
        if p.expires <= now() {
            let _ = b.delete(&key(store));
            return None;
        }
        Some(p)
    }

    pub fn status(store: &Store) -> Status {
        let p = load(store);
        Status {
            unlocked: p.is_some(),
            expires_in_seconds: p.map(|p| p.expires.saturating_sub(now())),
        }
    }

    /// The identity of an unlocked session.
    pub fn identity(store: &Store) -> Option<x25519::Identity> {
        parse_identity(load(store)?.identity.as_bytes()).ok()
    }

    pub fn require(store: &Store) -> Result<x25519::Identity> {
        identity(store).ok_or_else(super::locked)
    }
}

pub fn is_unlocked(store: &Store) -> bool {
    session::status(store).unlocked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_codes_are_grouped_and_normalise_back() {
        let c = recovery_code();
        assert_eq!(c.len(), 32 + 7);
        assert_eq!(normalise_code(&c.to_lowercase()), c.replace('-', ""));
        assert_ne!(c, recovery_code());
    }
}
