//! ADR 0002 / spec §3: dotwarden's git applies identity and signing settings from
//! the user's global config and nothing else, and never the system config.

mod common;

use common::TestEnv;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A global git config that would change behaviour if it were applied: hooks,
/// a filesystem monitor, a URL rewrite and an alias, next to a real identity
/// and signing settings. Returns the directory the markers are written to.
fn hostile_global_config(t: &TestEnv, rewrite_prefix: Option<&str>) -> PathBuf {
    let marks = t.root.path().canonicalize().unwrap().join("marks");
    fs::create_dir_all(&marks).unwrap();
    let hooks = marks.join("hooks");
    for hook in ["post-commit", "pre-commit", "post-checkout", "post-merge"] {
        script(
            &hooks.join(hook),
            &format!("echo ran > '{}/hook-{hook}'", marks.display()),
        );
    }
    let fsmonitor = marks.join("fsmonitor.sh");
    script(
        &fsmonitor,
        &format!("echo ran > '{}/fsmonitor'; exit 0", marks.display()),
    );
    let mut cfg = format!(
        "[user]\n\tname = Test User\n\temail = test@example.com\n\tsigningkey = ABC123\n\
         [commit]\n\tgpgsign = false\n[gpg]\n\tformat = ssh\n\
         [gpg \"ssh\"]\n\tprogram = /usr/bin/true\n\
         [core]\n\thooksPath = {}\n\tfsmonitor = {}\n\
         [alias]\n\tzz = status\n[init]\n\tdefaultBranch = hostile\n",
        hooks.display(),
        fsmonitor.display()
    );
    if let Some(prefix) = rewrite_prefix {
        cfg.push_str(&format!(
            "[url \"file:///nonexistent-mirror/\"]\n\tinsteadOf = {prefix}\n"
        ));
    }
    fs::write(t.home.join(".gitconfig"), cfg).unwrap();
    marks
}

fn no_markers(marks: &Path) {
    let found: Vec<_> = fs::read_dir(marks)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "hooks" && n != "fsmonitor.sh")
        .collect();
    assert!(
        found.is_empty(),
        "the user's global git config was applied: {found:?}"
    );
}

#[test]
fn init_commit_and_status_ignore_hooks_and_fsmonitor_from_the_global_config() {
    let t = TestEnv::new();
    let marks = hostile_global_config(&t, None);
    t.dw_ok(&["init", "--name", "hostile"]);
    t.dw_ok(&["status"]);
    t.dw_ok(&["doctor"]);
    no_markers(&marks);
    let repo = t.dirs().repo();
    // The branch comes from --branch, not from the user's init.defaultBranch.
    assert_eq!(t.git(&repo, &["symbolic-ref", "--short", "HEAD"]), "main");
    // Identity still comes from the global config.
    assert_eq!(
        t.git(&repo, &["log", "-1", "--format=%an <%ae>"]),
        "Test User <test@example.com>"
    );
}

#[test]
fn clone_ignores_url_rewrites_from_the_global_config() {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    let upstream = a.dirs().repo();
    let url = format!("file://{}", upstream.display());

    let b = TestEnv::new();
    let marks = hostile_global_config(&b, Some(&format!("file://{}", upstream.display())));
    // If insteadOf applied, this URL would point at /nonexistent-mirror/ and fail.
    b.dw_ok(&["clone", &url, "--name", "beta"]);
    no_markers(&marks);
    assert_eq!(
        b.git(&b.dirs().repo(), &["remote", "get-url", "origin"]),
        url
    );
}

#[test]
fn only_identity_and_signing_settings_are_forwarded() {
    let t = TestEnv::new();
    hostile_global_config(&t, None);
    t.dw_ok(&["init", "--name", "fwd"]);
    let repo = t.dirs().repo();
    let get = |key: &str| {
        let mut c = t.store().git_command(&repo);
        c.args(["config", "--get", key]);
        let o = c.output().unwrap();
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    assert_eq!(get("user.name").as_deref(), Some("Test User"));
    assert_eq!(get("user.email").as_deref(), Some("test@example.com"));
    assert_eq!(get("user.signingkey").as_deref(), Some("ABC123"));
    assert_eq!(get("commit.gpgsign").as_deref(), Some("false"));
    assert_eq!(get("gpg.format").as_deref(), Some("ssh"));
    assert_eq!(get("gpg.ssh.program").as_deref(), Some("/usr/bin/true"));
    for key in [
        "core.hooksPath",
        "core.fsmonitor",
        "alias.zz",
        "init.defaultBranch",
    ] {
        assert_eq!(get(key), None, "{key} must not be applied");
    }
}

#[test]
fn system_config_and_git_environment_never_apply_but_the_users_global_file_is_honoured() {
    let mut t = TestEnv::new();
    let evil = t.root.path().canonicalize().unwrap().join("evil.gitconfig");
    fs::write(
        &evil,
        "[core]\n\thooksPath = /evil\n[alias]\n\tzz = status\n",
    )
    .unwrap();
    let identity = t
        .root
        .path()
        .canonicalize()
        .unwrap()
        .join("identity.gitconfig");
    fs::write(
        &identity,
        "[user]\n\tname = Env User\n\temail = env@example.com\n[core]\n\thooksPath = /evil\n",
    )
    .unwrap();
    t.set("GIT_CONFIG_SYSTEM", evil.clone())
        .set("GIT_CONFIG_GLOBAL", identity)
        .set("GIT_CONFIG_COUNT", "1")
        .set("GIT_CONFIG_KEY_0", "core.hooksPath")
        .set("GIT_CONFIG_VALUE_0", "/evil")
        .set("GIT_CONFIG_PARAMETERS", "'core.hooksPath=/evil'")
        .set("GIT_DIR", "/nonexistent");
    t.dw_ok(&["init", "--name", "envs"]);
    let repo = t.dirs().repo();
    let mut c = t.store().git_command(&repo);
    c.args([
        "config",
        "--get-regexp",
        "^(core\\.(hookspath|fsmonitor)|alias\\.)",
    ]);
    let o = c.output().unwrap();
    assert!(
        !o.status.success() || String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        "unexpected config: {}",
        String::from_utf8_lossy(&o.stdout)
    );
    // The identity comes from the file GIT_CONFIG_GLOBAL points at.
    assert_eq!(
        t.git(&repo, &["log", "-1", "--format=%an <%ae>"]),
        "Env User <env@example.com>"
    );
}

#[test]
fn ssh_settings_pass_through_to_git() {
    let mut t = TestEnv::new();
    let marker = t.root.path().canonicalize().unwrap().join("ssh-ran");
    let ssh = t.root.path().canonicalize().unwrap().join("fake-ssh");
    script(
        &ssh,
        &format!(
            "echo \"sock=$SSH_AUTH_SOCK args=$*\" > '{}'; exit 255",
            marker.display()
        ),
    );
    t.set("GIT_SSH_COMMAND", ssh.display().to_string())
        .set("SSH_AUTH_SOCK", "/tmp/dw-fake-agent.sock");
    let mut c = t.store().git_command(&t.home);
    c.args(["ls-remote", "ssh://git@example.invalid/repo.git"]);
    let o = c.output().unwrap();
    assert!(!o.status.success());
    let seen = fs::read_to_string(&marker).expect("GIT_SSH_COMMAND was not used");
    assert!(seen.contains("sock=/tmp/dw-fake-agent.sock"), "{seen}");
    assert!(seen.contains("example.invalid"), "{seen}");
}

#[test]
fn chezmoi_runs_with_the_bundled_git_and_the_same_isolation() {
    let t = TestEnv::new();
    hostile_global_config(&t, None);
    t.dw_ok(&["init", "--name", "cz"]);
    let store = t.store();
    let src = t.dirs().repo().join("home");
    fs::write(
        src.join("dot_gv.tmpl"),
        "{{ output \"git\" \"--version\" | trim }}",
    )
    .unwrap();
    fs::write(
        src.join("dot_gcfg.tmpl"),
        "{{ output \"sh\" \"-c\" \"git config --get user.email; git config --get core.hooksPath || echo no-hooks\" | trim }}",
    )
    .unwrap();
    let cat = |name: &str| {
        let path = t.home.join(name);
        store
            .chezmoi(&["cat".as_ref(), path.as_os_str()])
            .unwrap()
            .stdout
    };
    // The bundled git is first on PATH, ahead of any system git.
    assert_eq!(
        cat(".gv"),
        format!("git version {}", dotwarden::pins::git_version())
    );
    // chezmoi's git sees the forwarded identity and not the rest of ~/.gitconfig.
    assert_eq!(cat(".gcfg"), "test@example.com\nno-hooks");
}

/// A credential helper script that records each call and answers with fixed credentials.
fn helper_config(t: &TestEnv) -> PathBuf {
    let dir = t.root.path().canonicalize().unwrap();
    let helper = dir.join("cred-helper.sh");
    script(
        &helper,
        &format!(
            "echo \"$1\" >> '{}/helper-calls'; cat > /dev/null; printf 'username=x\\npassword=y\\n'",
            dir.display()
        ),
    );
    let cfg = t.home.join(".gitconfig");
    let mut text = fs::read_to_string(&cfg).unwrap_or_default();
    text.push_str(&format!(
        "[credential]\n\thelper = !{}\n[alias]\n\tzz = status\n",
        helper.display()
    ));
    fs::write(&cfg, text).unwrap();
    dir.join("helper-calls")
}

fn credential_fill(mut c: std::process::Command) -> (String, bool) {
    use std::io::Write;
    c.args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=example.invalid\n\n")
        .unwrap();
    let o = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        o.status.success(),
    )
}

#[test]
fn the_users_credential_helper_serves_dotwardens_git_but_not_the_anonymous_probe() {
    let t = TestEnv::new();
    let calls = helper_config(&t);
    let store = t.store();

    let (out, ok) = credential_fill(store.git_command(&t.home));
    assert!(
        ok && out.contains("username=x") && out.contains("password=y"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(&calls).unwrap().trim(), "get");

    // Other global keys stay blocked.
    let mut c = store.git_command(&t.home);
    c.args(["config", "--get", "alias.zz"]);
    assert!(!c.output().unwrap().status.success());

    // The anonymous command carries no helper, so nothing is called or answered.
    fs::remove_file(&calls).unwrap();
    let mut anon = store.git_command_anonymous(&t.home);
    anon.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/bin/false");
    let (out, ok) = credential_fill(anon);
    assert!(!ok && !out.contains("password=y"), "{out}");
    assert!(!calls.exists(), "the helper ran for an anonymous command");
}

#[test]
fn credential_entries_keep_order_and_empty_resets_and_skip_unrelated_keys() {
    let t = TestEnv::new();
    fs::write(
        t.home.join(".gitconfig"),
        "[user]\n\tname = A\n\temail = a@example.com\n\
         [credential]\n\thelper = first\n\tuseHttpPath = true\n\
         [credential \"https://github.com\"]\n\thelper =\n\thelper = !gh auth git-credential\n\
         \tusername = me\n\tcache = nope\n\
         [credential]\n\thelper = last\n\
         [alias]\n\tzz = status\n[url \"ssh://x/\"]\n\tinsteadOf = https://y/\n\
         [core]\n\thooksPath = /evil\n",
    )
    .unwrap();
    let got = t.store().git_credential_entries();
    let got: Vec<(&str, &str)> = got.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(
        got,
        [
            ("credential.helper", "first"),
            ("credential.usehttppath", "true"),
            ("credential.https://github.com.helper", ""),
            (
                "credential.https://github.com.helper",
                "!gh auth git-credential"
            ),
            ("credential.https://github.com.username", "me"),
            ("credential.helper", "last"),
        ]
    );
}

#[test]
fn the_public_remote_probe_carries_no_credential_helper() {
    let t = TestEnv::new();
    let calls = helper_config(&t);
    let store = t.store();
    let cmd = dotwarden::remote::probe_command(&store, "https://example.invalid/me/d.git");
    let env: Vec<(String, String)> = cmd
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
        .collect();
    let keys: Vec<&str> = env
        .iter()
        .filter(|(k, _)| k.starts_with("GIT_CONFIG_KEY_"))
        .map(|(_, v)| v.as_str())
        .collect();
    assert!(keys.contains(&"user.name"), "{keys:?}");
    assert!(
        keys.iter().all(|k| !k.starts_with("credential.")),
        "{keys:?}"
    );
    assert!(cmd.get_args().any(|a| a == "credential.helper="));
    assert!(env.contains(&("GIT_TERMINAL_PROMPT".into(), "0".into())));
    assert!(!calls.exists());
}
