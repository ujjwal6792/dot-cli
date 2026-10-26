#![allow(dead_code)]
//! Shared helpers: an isolated HOME with its own XDG directories, global git
//! identity, and the bundled tools from the checkout's `libexec/`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dotwarden::paths::{Dirs, Env};
use dotwarden::store::Store;
use dotwarden::toolchain::Toolchain;

pub fn libexec() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("libexec");
    assert!(
        p.join("chezmoi").is_file() && p.join("git/bin/git").is_file(),
        "bundled tools missing in {}: run scripts/fetch-tools",
        p.display()
    );
    p
}

pub struct TestEnv {
    pub root: tempfile::TempDir,
    pub home: PathBuf,
    pub env: Env,
}

impl TestEnv {
    pub fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("dw-test-")
            .tempdir()
            .unwrap();
        // Canonicalise: macOS temp dirs live behind the /var -> /private/var link.
        let base = root.path().canonicalize().unwrap();
        let home = base.join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Test User\n\temail = test@example.com\n[init]\n\tdefaultBranch = trunk\n",
        )
        .unwrap();
        let mut env = Env::new();
        let path = std::env::var_os("PATH").unwrap_or_default();
        env.insert("PATH".into(), path);
        env.insert("HOME".into(), home.clone().into());
        env.insert("LANG".into(), "C".into());
        env.insert("DOTWARDEN_LIBEXEC".into(), libexec().into());
        Self { root, home, env }
    }

    pub fn set(&mut self, k: &str, v: impl Into<OsString>) -> &mut Self {
        self.env.insert(k.into(), v.into());
        self
    }

    pub fn dirs(&self) -> Dirs {
        Dirs::from_env(&self.env).unwrap()
    }

    pub fn store(&self) -> Store {
        Store::new(self.dirs(), Toolchain::at(libexec()), self.env.clone())
    }

    /// Run the `dotwarden` binary in this environment (stdin is null).
    pub fn dw(&self, args: &[&str]) -> Output {
        self.run(env!("CARGO_BIN_EXE_dotwarden"), args)
    }

    pub fn run(&self, bin: &str, args: &[&str]) -> Output {
        Command::new(bin)
            .args(args)
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.home)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    pub fn dw_ok(&self, args: &[&str]) -> String {
        let o = self.dw(args);
        assert!(
            o.status.success(),
            "dw {args:?} failed: {}\n{}",
            String::from_utf8_lossy(&o.stderr),
            String::from_utf8_lossy(&o.stdout)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    pub fn dw_json(&self, args: &[&str]) -> serde_json::Value {
        let mut a = args.to_vec();
        a.push("--json");
        serde_json::from_str(&self.dw_ok(&a)).expect("valid JSON")
    }

    /// Run bundled git in `dir` and return trimmed stdout.
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        let mut c = self.store().git_command(dir);
        c.args(args);
        let o = c.output().unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }
}

/// Recursively record `(relative path, size, mtime, contents)` of everything
/// under `dir`; an absent directory yields `None`.
pub fn snapshot(dir: &Path) -> Option<Vec<String>> {
    if !dir.exists() {
        return None;
    }
    let mut out = Vec::new();
    fn walk(base: &Path, p: &Path, out: &mut Vec<String>) {
        let meta = fs::symlink_metadata(p).unwrap();
        let rel = p.strip_prefix(base).unwrap().display().to_string();
        let mtime = meta.modified().unwrap();
        let body = if meta.is_file() {
            fs::read(p).unwrap()
        } else {
            vec![]
        };
        out.push(format!(
            "{rel} len={} mtime={mtime:?} body={body:?}",
            meta.len()
        ));
        if meta.is_dir() {
            let mut kids: Vec<_> = fs::read_dir(p)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .collect();
            kids.sort();
            for k in kids {
                walk(base, &k, out);
            }
        }
    }
    walk(dir, dir, &mut out);
    Some(out)
}
