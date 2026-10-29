//! The vault with real chezmoi and git: crypto lifecycle, secrets in files,
//! the helper, encrypted files, detection and masking. Sessions use the file
//! backend in a temp XDG_RUNTIME_DIR (never the real keychain); passphrases
//! come through the debug-only `DOTWARDEN_TEST_*` hooks.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;

use common::TestEnv;

pub const PASS: &str = "correct horse battery";

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

impl Run {
    fn all(&self) -> String {
        format!("{}{}", self.out, self.err)
    }
}

fn run(t: &TestEnv, extra: &[(&str, &str)], agent: bool, args: &[&str]) -> Run {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"));
    c.args(args)
        .env_clear()
        .envs(&t.env)
        .envs(extra.iter().copied())
        .current_dir(&t.home)
        .stdin(Stdio::null());
    if agent {
        c.env("DOTWARDEN_AGENT", "claude");
    }
    let o = c.output().unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    run(t, &[], false, args)
}

fn ok(t: &TestEnv, extra: &[(&str, &str)], args: &[&str]) -> String {
    let r = run(t, extra, false, args);
    assert_eq!(r.code, 0, "dw {args:?}: {}{}", r.out, r.err);
    r.out
}

fn env() -> TestEnv {
    let mut t = TestEnv::new();
    let rt = t.root.path().canonicalize().unwrap().join("rt");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_SESSION_BACKEND", "file");
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn read(t: &TestEnv, rel: &str) -> String {
    fs::read_to_string(t.home.join(rel)).unwrap()
}

/// Returns the recovery code.
fn init_vault(t: &TestEnv) -> String {
    let out = ok(
        t,
        &[("DOTWARDEN_TEST_NEW_PASSPHRASE", PASS)],
        &["vault", "init", "--json"],
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    v["recovery_code"].as_str().unwrap().to_string()
}

fn unlock(t: &TestEnv) {
    ok(
        t,
        &[("DOTWARDEN_TEST_PASSPHRASE", PASS)],
        &["vault", "unlock"],
    );
}

fn add_secret(t: &TestEnv, name: &str, target: Option<&str>, value: &str) -> Run {
    let mut a = vec!["values", "add-secret", name];
    if let Some(x) = target {
        a.extend(["--target", x]);
    }
    run(t, &[("DOTWARDEN_TEST_SECRET_VALUE", value)], false, &a)
}

fn bare(t: &TestEnv) -> PathBuf {
    let p = t.root.path().join("remote.git");
    t.git(
        &t.home,
        &[
            "init",
            "--bare",
            "--quiet",
            "--initial-branch=main",
            p.to_str().unwrap(),
        ],
    );
    p
}

/// mac (the first machine, vault created and unlocked) and srv (a linux
/// machine cloned from it, locked), sharing a bare remote.
fn pair() -> (TestEnv, TestEnv) {
    let a = env();
    a.dw_ok(&["init", "--name", "mac"]);
    init_vault(&a);
    a.dw_ok(&["commit", "-m", "vault"]);
    let remote = bare(&a);
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = env();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "srv"]);
    let file = repo(&b).join("home/.chezmoidata/dotwarden-machines/srv.toml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("\"darwin\"", "\"linux\"")
        .replace("\"macos\"", "\"ubuntu\"");
    fs::write(&file, text).unwrap();
    b.git(&repo(&b), &["add", "-A"]);
    b.dw_ok(&["commit", "-m", "srv is linux"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);
    (a, b)
}

fn sync(from: &TestEnv, to: &TestEnv) {
    from.dw_ok(&["push", "--yes"]);
    to.dw_ok(&["pull"]);
}

#[test]
fn passphrase_lifecycle_passwd_and_recovery() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    let code = init_vault(&t);
    assert!(ok(&t, &[], &["vault", "status"]).contains("unlocked"));
    ok(&t, &[], &["vault", "lock"]);
    assert!(ok(&t, &[], &["vault", "status"]).contains("locked"));
    let wrong = run(
        &t,
        &[("DOTWARDEN_TEST_PASSPHRASE", "nope nope nope")],
        false,
        &["vault", "unlock"],
    );
    assert_ne!(wrong.code, 0);
    assert!(ok(&t, &[], &["vault", "status"]).contains("locked"));
    unlock(&t);
    assert!(ok(&t, &[], &["vault", "status"]).contains("unlocked"));

    let v = repo(&t).join("home/.dotwarden/vault");
    let (rec_before, rcp_before) = (
        fs::read(v.join("recovery.age")).unwrap(),
        fs::read(v.join("recipient.txt")).unwrap(),
    );
    let id_before = fs::read(v.join("identity.age")).unwrap();
    ok(
        &t,
        &[
            ("DOTWARDEN_TEST_PASSPHRASE", PASS),
            ("DOTWARDEN_TEST_NEW_PASSPHRASE", "second passphrase"),
        ],
        &["vault", "passwd"],
    );
    assert_ne!(fs::read(v.join("identity.age")).unwrap(), id_before);
    assert_eq!(
        fs::read(v.join("recovery.age")).unwrap(),
        rec_before,
        "passwd rewrites identity.age only"
    );
    assert_eq!(fs::read(v.join("recipient.txt")).unwrap(), rcp_before);
    ok(&t, &[], &["vault", "lock"]);
    assert_ne!(
        run(
            &t,
            &[("DOTWARDEN_TEST_PASSPHRASE", PASS)],
            false,
            &["vault", "unlock"]
        )
        .code,
        0
    );
    ok(
        &t,
        &[("DOTWARDEN_TEST_PASSPHRASE", "second passphrase")],
        &["vault", "unlock"],
    );

    ok(&t, &[], &["vault", "lock"]);
    ok(
        &t,
        &[
            ("DOTWARDEN_TEST_RECOVERY_CODE", &code.to_lowercase()),
            ("DOTWARDEN_TEST_NEW_PASSPHRASE", "third passphrase"),
        ],
        &["vault", "recover"],
    );
    assert!(ok(&t, &[], &["vault", "status"]).contains("unlocked"));
    ok(&t, &[], &["vault", "lock"]);
    ok(
        &t,
        &[("DOTWARDEN_TEST_PASSPHRASE", "third passphrase")],
        &["vault", "unlock"],
    );
}

#[test]
fn secrets_are_never_read_from_arguments_environment_or_pipes() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    // No terminal: refused (usage, exit 2) even with look-alike variables and piped stdin.
    let refuse_piped = |args: &[&str]| {
        let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"));
        c.args(args)
            .env_clear()
            .envs(&t.env)
            .env("DOTWARDEN_PASSPHRASE", PASS)
            .env("PASSPHRASE", PASS)
            .current_dir(&t.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        use std::io::Write;
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{PASS}\n{PASS}\n").as_bytes());
        let o = child.wait_with_output().unwrap();
        assert_eq!(
            o.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        assert!(String::from_utf8_lossy(&o.stderr).contains("interactive terminal"));
    };
    refuse_piped(&["vault", "init"]);
    assert!(
        !repo(&t)
            .join("home/.dotwarden/vault/recipient.txt")
            .exists(),
        "nothing was created"
    );
    // An agent is refused even with the test hook.
    let r = run(
        &t,
        &[("DOTWARDEN_TEST_NEW_PASSPHRASE", PASS)],
        true,
        &["vault", "init"],
    );
    assert_eq!(r.code, 2);
    init_vault(&t);
    ok(&t, &[], &["vault", "lock"]);
    refuse_piped(&["vault", "unlock"]);
    assert!(ok(&t, &[], &["vault", "status"]).contains("locked"));
    // A value is not an argument either.
    let r = dw(&t, &["values", "add-secret", "TOKEN", "hunter2hunter2"]);
    assert_eq!(r.code, 2);
    let r = dw(&t, &["values", "add-secret", "TOKEN"]);
    assert_eq!(r.code, 2, "no terminal: {}", r.err);
}

#[test]
fn agents_add_placeholders_and_wire_but_never_set_or_see_values() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    init_vault(&t);
    let secret = "S3cretValue-Zq8x7Lm2";
    assert_eq!(add_secret(&t, "DB_PASS", None, secret).code, 0);
    let a = |args: &[&str]| run(&t, &[("DOTWARDEN_TEST_SECRET_VALUE", secret)], true, args);
    // Setting a value is refused for an agent even with the hook; a placeholder is fine.
    assert_eq!(a(&["values", "add-secret", "OTHER"]).code, 2);
    assert_eq!(
        a(&["values", "add-secret", "OTHER", "--placeholder"]).code,
        0
    );
    assert!(read_meta(&t, "OTHER").contains("placeholder = true"));
    // Every inspection command, as an agent, as JSON and human: no value.
    for args in [
        &["values", "list"][..],
        &["values", "show", "DB_PASS"][..],
        &["status"][..],
        &["diff"][..],
        &["pending"][..],
        &["vault", "status"][..],
        &["ls"][..],
    ] {
        for json in [false, true] {
            let mut v = args.to_vec();
            if json {
                v.push("--json");
            }
            let r = a(&v);
            assert!(!r.all().contains(secret), "{v:?} leaked");
        }
    }
    let list = a(&["values", "list", "--json"]).out;
    assert!(list.contains("DB_PASS") && list.contains("secret") && list.contains("••••••"));
    // An agent cannot unlock and the helper refuses without a token.
    assert_ne!(a(&["vault", "unlock"]).code, 0);
    let h = run(
        &t,
        &[("DOTWARDEN_RENDER_TOKEN", &"a".repeat(64))],
        false,
        &["__secret", "DB_PASS", "[\"platform:darwin\"]"],
    );
    assert_ne!(h.code, 0);
    assert!(!h.all().contains(secret));
    let h = dw(&t, &["__secret", "DB_PASS", "[]"]);
    assert_ne!(h.code, 0);
    assert!(h.out.is_empty());
}

fn read_meta(t: &TestEnv, name: &str) -> String {
    fs::read_to_string(repo(t).join(format!("home/.chezmoidata/dotwarden-values/{name}.toml")))
        .unwrap()
}

const DEFAULT_V: &str = "Default-Val-9a8b7c6d";
const SRV_V: &str = "Srv-Only-Val-1f2e3d4c";

/// `.conf` wired to secret API on mac (default + a machine:srv override).
fn wired_pair() -> (TestEnv, TestEnv) {
    let (a, b) = pair();
    write(&a, ".conf", &format!("key = {DEFAULT_V}\nname = x\n"));
    // Detection would propose moving the value; set the secret first instead.
    a.dw_ok(&["track", ".conf", "--apply"]);
    assert_eq!(add_secret(&a, "API", None, DEFAULT_V).code, 0);
    a.dw_ok(&["values", "wire", ".conf", "API", "--apply"]);
    assert_eq!(add_secret(&a, "API", Some("machine:srv"), SRV_V).code, 0);
    a.dw_ok(&["commit", "-m", "api"]);
    sync(&a, &b);
    (a, b)
}

#[test]
fn two_machines_each_receive_their_own_secret_after_unlocking() {
    let (a, b) = wired_pair();
    let stored = fs::read_to_string(repo(&a).join("home/dot_conf.tmpl")).unwrap();
    assert!(stored.contains("\"API\"") && !stored.contains(DEFAULT_V));
    assert_eq!(read(&a, ".conf"), format!("key = {DEFAULT_V}\nname = x\n"));
    assert!(!a.dw_ok(&["status"]).contains("Local"));

    // srv is locked: the file is skipped (exit 0 for --all, exit 4 when named).
    let r = dw(&b, &["apply", "--all"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.out.contains("vault locked"), "{}", r.out);
    assert!(!b.home.join(".conf").exists());
    let r = dw(&b, &["apply", ".conf"]);
    assert_eq!(r.code, 4, "{}", r.all());
    assert!(b.dw_ok(&["status"]).contains("Vault locked"));

    unlock(&b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".conf"), format!("key = {SRV_V}\nname = x\n"));
    assert!(!b.dw_ok(&["status"]).contains("Local"));
    // Locking again hides it from classification but removes nothing.
    ok(&b, &[], &["vault", "lock"]);
    assert_eq!(read(&b, ".conf"), format!("key = {SRV_V}\nname = x\n"));
}

#[test]
fn an_edited_secret_is_masked_then_flows_into_the_vault_keeping_the_reference() {
    let (a, b) = wired_pair();
    let new = "Edited-Val-77aa66bb";
    write(&a, ".conf", &format!("key = {new}\nname = y\n"));
    let st = a.dw_ok(&["status"]);
    assert!(
        st.contains("Secret API changed") && !st.contains(new) && !st.contains(DEFAULT_V),
        "{st}"
    );
    let diff = a.dw_ok(&["diff"]);
    assert!(
        diff.contains("«secret:API»") && !diff.contains(new) && !diff.contains(DEFAULT_V),
        "{diff}"
    );
    assert!(diff.contains("name = y"), "{diff}");
    // Locked: staging that file asks for an unlock (exit 4).
    ok(&a, &[], &["vault", "lock"]);
    let r = dw(&a, &["stage", ".conf"]);
    assert_eq!(r.code, 4, "{}", r.all());
    unlock(&a);
    a.dw_ok(&["stage", ".conf"]);
    assert!(
        fs::read_to_string(repo(&a).join("home/dot_conf.tmpl"))
            .unwrap()
            .contains("name = y")
    );
    a.dw_ok(&["commit", "-m", "edit"]);
    sync(&a, &b);
    unlock(&b);
    b.dw_ok(&["apply", "--all"]);
    // srv keeps its own override; mac's new default reaches mac's vault only.
    assert_eq!(read(&b, ".conf"), format!("key = {SRV_V}\nname = y\n"));
    fs::remove_file(a.home.join(".conf")).unwrap();
    a.dw_ok(&["discard", ".conf", "--yes"]);
    assert_eq!(read(&a, ".conf"), format!("key = {new}\nname = y\n"));
}

#[test]
fn an_encrypted_ssh_key_round_trips_with_its_mode() {
    let (a, b) = pair();
    let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAAC3NzaC1lZDI1NTE5Plainkeymaterial0001\n-----END OPENSSH PRIVATE KEY-----\n";
    write(&a, ".ssh/id_test", key);
    fs::set_permissions(
        a.home.join(".ssh/id_test"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    a.dw_ok(&["track", ".ssh/id_test", "--apply"]);
    assert!(
        grep_everywhere(&a, "Plainkeymaterial").is_empty(),
        "never written, not just pruned"
    );
    a.dw_ok(&["commit", "-m", "key"]);
    let all = grep_everywhere(&a, "Plainkeymaterial");
    assert!(all.is_empty(), "plaintext found in {all:?}");
    assert!(!a.dw_ok(&["status"]).contains("Local"));
    sync(&a, &b);
    let r = dw(&b, &["apply", "--all"]);
    assert!(r.out.contains("vault locked"), "{}", r.out);
    unlock(&b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".ssh/id_test"), key);
    let mode = fs::metadata(b.home.join(".ssh/id_test"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    // Editing it and staging re-encrypts it (no unlock needed to encrypt, but to classify).
    write(&b, ".ssh/id_test", &key.replace("0001", "0002"));
    let d = b.dw_ok(&["diff"]);
    assert!(
        !d.contains("Plainkeymaterial") && d.contains("encrypted file"),
        "{d}"
    );
    b.dw_ok(&["stage", ".ssh/id_test"]);
    b.dw_ok(&["commit", "-m", "rotate"]);
    assert!(grep_everywhere(&b, "Plainkeymaterial").is_empty());
}

/// Files and git objects (all of them, loose or packed) that contain `needle`.
fn grep_everywhere(t: &TestEnv, needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut dirs = vec![
        repo(t).join("home"),
        t.dirs().state_dir(),
        t.dirs().data_dir().join("chezmoi.toml"),
    ];
    dirs.push(
        t.env
            .get(std::ffi::OsStr::new("XDG_RUNTIME_DIR"))
            .map(PathBuf::from)
            .unwrap(),
    );
    while let Some(p) = dirs.pop() {
        if let Ok(rd) = fs::read_dir(&p) {
            dirs.extend(rd.flatten().map(|e| e.path()));
        } else if fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle)) {
            found.push(p.display().to_string());
        }
    }
    let mut cmd = t.store().git_command(&repo(t));
    cmd.args(["cat-file", "--batch-all-objects", "--batch"]);
    let o = cmd.output().unwrap();
    if String::from_utf8_lossy(&o.stdout).contains(needle) {
        found.push("git objects".into());
    }
    found
}

#[test]
fn a_secret_found_on_track_never_reaches_the_repository_or_git_objects() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    init_vault(&t);
    t.dw_ok(&["commit", "-m", "vault"]);
    let found = "Zq8x7Lm2Pq9wRt5vYb3nKd6HfJs1AaGc";
    write(&t, ".env", &format!("host = h\napi_token = {found}\n"));
    let dry = t.dw_ok(&["track", ".env", "--dry-run"]);
    assert!(
        dry.contains("holds a secret") && !dry.contains(found),
        "{dry}"
    );
    let out = t.dw_ok(&["track", ".env", "--apply"]);
    assert!(!out.contains(found));
    // Not even a loose blob was created: nothing to prune.
    assert!(
        grep_everywhere(&t, found).is_empty(),
        "{:?}",
        grep_everywhere(&t, found)
    );
    let stored = fs::read_to_string(repo(&t).join("home/dot_env.tmpl")).unwrap();
    assert!(stored.contains("includeTemplate") && !stored.contains(found));
    t.dw_ok(&["commit", "-m", "env"]);
    assert_eq!(
        read(&t, ".env"),
        format!("host = h\napi_token = {found}\n"),
        "HOME untouched"
    );
    assert!(!t.dw_ok(&["status"]).contains("Local"));
    assert!(t.dw_ok(&["values", "list"]).contains("API_TOKEN"));
    // The same value in another file is matched against the existing secret.
    write(&t, ".other", &format!("x={found}\n"));
    t.dw_ok(&["track", ".other", "--apply"]);
    let other = fs::read_to_string(repo(&t).join("home/dot_other.tmpl")).unwrap();
    assert!(
        other.contains("\"API_TOKEN\"") && !other.contains(found),
        "{other}"
    );
    // Staging an edit that adds a secret to a tracked plain file is sanitized too.
    write(&t, ".plain", "a = 1\n");
    t.dw_ok(&["track", ".plain", "--apply"]);
    t.dw_ok(&["commit", "-m", "plain"]);
    let second = "Mm4Nn5Bb6Vv7Cc8Xx9Zz0Aa1Ss2Dd3Ff";
    write(&t, ".plain", &format!("a = 1\npassword = {second}\n"));
    t.dw_ok(&["stage", ".plain"]);
    assert!(grep_everywhere(&t, second).is_empty());
    assert!(
        repo(&t).join("home/dot_plain.tmpl").is_file() && !repo(&t).join("home/dot_plain").exists()
    );
}

#[test]
fn without_a_vault_a_detected_secret_is_refused_before_anything_is_written() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    let key = "fc-1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d";
    write(&t, ".zshrc", &format!("export FIRECRAWL_API_KEY={key}\n"));
    let r = dw(&t, &["track", ".zshrc", "--apply"]);
    assert_eq!(r.code, 1, "{}", r.all());
    assert!(r.all().contains("dw vault init") && !r.all().contains(key));
    assert!(grep_everywhere(&t, key).is_empty());
    assert!(!repo(&t).join("home/dot_zshrc").exists());
    // An agent cannot opt out; a human can.
    let a = run(
        &t,
        &[],
        true,
        &["track", ".zshrc", "--allow-plaintext", "--apply"],
    );
    assert_eq!(a.code, 2);
    t.dw_ok(&["track", ".zshrc", "--allow-plaintext", "--apply"]);
    assert!(repo(&t).join("home/dot_zshrc").is_file());
}

#[test]
fn canary_no_command_log_state_file_or_git_object_holds_a_secret_value() {
    let (a, b) = wired_pair();
    let canary = "CANARY-7f3a9c1e-5b2d";
    assert_eq!(add_secret(&a, "CANARY", None, canary).code, 0);
    write(&a, ".canary", &format!("c = {canary}\n"));
    a.dw_ok(&["track", ".canary", "--apply"]);
    a.dw_ok(&["commit", "-m", "canary"]);
    write(&a, ".canary", &format!("c = {canary}-changed\nd = 1\n"));
    let mut outputs = String::new();
    let cmds: &[&[&str]] = &[
        &["status"],
        &["diff"],
        &["ls"],
        &["pending"],
        &["values", "list"],
        &["values", "show", "CANARY"],
        &["vault", "status"],
        &["machine", "list"],
        &["snapshot", "list"],
        &["doctor"],
        &["variant", "list"],
        &["stage", "--dry-run"],
        &["apply", "--all", "--dry-run"],
        &["values", "wire", ".canary", "CANARY", "--dry-run"],
        &["values", "add-secret", "CANARY", "--dry-run"],
        &["encrypt", ".canary", "--dry-run"],
        &["commit", "-m", "x", "--dry-run"],
    ];
    for c in cmds {
        for json in [false, true] {
            let mut v = c.to_vec();
            if json {
                v.push("--json");
            }
            let r = run(&a, &[], false, &v);
            outputs.push_str(&r.all());
        }
        // Locked too.
    }
    ok(&a, &[], &["vault", "lock"]);
    for c in cmds {
        outputs.push_str(&run(&a, &[], false, c).all());
        outputs.push_str(&run(&a, &[], true, c).all());
    }
    assert!(!outputs.contains(canary), "a command printed the secret");
    assert!(outputs.contains("«secret:CANARY»") || outputs.contains("CANARY"));
    // Stage (unlocked) keeps the vault current; nothing leaks into any file.
    unlock(&a);
    a.dw_ok(&["stage", ".canary"]);
    a.dw_ok(&["commit", "-m", "changed"]);
    sync(&a, &b);
    for t in [&a, &b] {
        let hits = grep_everywhere(t, canary);
        assert!(hits.is_empty(), "plaintext in {hits:?}");
    }
}

#[test]
fn wiring_a_secret_compares_the_real_value_and_needs_the_unlock() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    init_vault(&t);
    let v = "fc-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    assert_eq!(add_secret(&t, "FIRECRAWL_API_KEY", None, v).code, 0);
    t.dw_ok(&["commit", "-m", "vault"]);
    write(&t, ".zshrc", &format!("export FIRECRAWL_API_KEY={v}\n"));
    t.dw_ok(&["track", ".zshrc", "--allow-plaintext", "--apply"]);
    ok(&t, &[], &["vault", "lock"]);
    let r = dw(
        &t,
        &["values", "wire", ".zshrc", "FIRECRAWL_API_KEY", "--apply"],
    );
    assert_eq!(r.code, 4, "{}", r.all());
    unlock(&t);
    let dry = t.dw_ok(&["values", "wire", ".zshrc", "FIRECRAWL_API_KEY", "--dry-run"]);
    assert!(
        dry.contains("«secret:FIRECRAWL_API_KEY»") && !dry.contains(v),
        "{dry}"
    );
    t.dw_ok(&["values", "wire", ".zshrc", "FIRECRAWL_API_KEY", "--apply"]);
    let stored = fs::read_to_string(repo(&t).join("home/dot_zshrc.tmpl")).unwrap();
    assert!(stored.contains("\"FIRECRAWL_API_KEY\"") && !stored.contains(v));
    // A file without the value says so without printing anything secret.
    write(&t, ".other", "nothing\n");
    t.dw_ok(&["track", ".other", "--apply"]);
    let r = dw(
        &t,
        &["values", "wire", ".other", "FIRECRAWL_API_KEY", "--apply"],
    );
    assert_eq!(r.code, 1);
    assert!(
        r.err.contains("does not occur") && !r.all().contains(v),
        "{}",
        r.all()
    );
}

#[test]
fn add_secret_has_apply_and_a_dry_run_that_changes_nothing() {
    let t = env();
    t.dw_ok(&["init", "--name", "mac"]);
    init_vault(&t);
    let h = [("DOTWARDEN_TEST_SECRET_VALUE", "Dry-Run-Value-123")];
    let dry: serde_json::Value = serde_json::from_str(&ok(
        &t,
        &h,
        &["values", "add-secret", "NEW", "--dry-run", "--json"],
    ))
    .unwrap();
    assert_eq!(dry["dry_run"], true);
    assert!(
        !repo(&t)
            .join("home/.dotwarden/vault/secrets/NEW.age")
            .exists()
    );
    assert!(!dry.to_string().contains("Dry-Run-Value"));
    ok(&t, &h, &["values", "add-secret", "NEW", "--apply"]);
    assert!(
        repo(&t)
            .join("home/.dotwarden/vault/secrets/NEW.age")
            .is_file()
    );
}
