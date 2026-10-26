# T1 report — Walking skeleton (branch `t1-sonnet`)

## What was built

- Cargo project (edition 2024): `dotwarden` and `dw` binaries (second `[[bin]]`) sharing a lib crate.
- **Toolchain** (`src/toolchain.rs`): resolves `$DOTWARDEN_LIBEXEC`, then `<exe>/../libexec`, then (debug builds) the checkout's git-ignored `libexec/`. Pins live in `toolchain/pins.env` (chezmoi 2.73.3, dugite-native v2.56.0-2 / git 2.56.0, delta 0.20.1; checksums cross-checked against upstream checksum files and release digests). `scripts/fetch-tools` downloads and SHA-256-verifies them for darwin-arm64, linux-arm64, linux-amd64. Dugite git env (`GIT_EXEC_PATH`, `GIT_TEMPLATE_DIR`) is set by the wrapper.
- **Isolated store** (`src/store.rs`): every chezmoi call passes `--config/--source/--persistent-state/--cache/--destination` and has `CHEZMOI_*` stripped. Git runs with `GIT_CONFIG_NOSYSTEM=1`, redirecting `GIT_*` variables removed, identity from the global gitconfig. XDG dirs respected.
- **`dw init [--name] [--branch]`, `dw clone <url> [--name] [--adopt]`**: spec §3 layout, `.chezmoiroot`=`home`, fixed `.chezmoiignore`, helper templates, default `hide`, `dotwarden.toml`. Machine registration (name from `--name` or prompt on a TTY; platform/distribution/families from os-release or macOS), registry entry in `home/.chezmoidata/dotwarden.toml`, local `machine.toml`, generated `chezmoi.toml`. Failures clean up the half-created repo.
- **`dw status --json`, `dw doctor`**, every flag/argument has help text (unit-tested).
- **TUI** (`src/tui/`): panels 1–5, main view, Pending pane, Files HOME browser (hide rules, `.` toggle, j/k/h/l/arrows), `?` help, `q`, resize, single-panel mode below 60×16. All I/O on a job runner (supersede-and-cancel by key); the event loop only waits on one channel.
- `scripts/pty-smoke.py`: the real binary on a PTY (start, hidden toggle, help, resize, quit).

## How to run

```
scripts/fetch-tools            # into ./libexec (git-ignored)
cargo build
export HOME=$(mktemp -d) ; git config --global user.name X ; git config --global user.email x@y   # never use the real HOME
target/debug/dw init --name mybox ; target/debug/dw status --json ; target/debug/dw doctor ; target/debug/dw
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
python3 -I scripts/pty-smoke.py target/debug/dotwarden
```

## Spike findings

Recorded in `docs/spec.md` §3 "Verified mechanics" with exact template code; each proven in `tests/spike.rs` and `tests/isolation.rs`. Highlights: `.chezmoiroot` works with the repo root as `--source`; leading-dot entries under `home/` are never targets; `include` paths are relative to `home/`; assignments (union includes, excludes win, four target kinds) evaluate in the fixed `.chezmoiignore` from registry data plus only the registry name in generated config; variant helper picks machine > distribution > family > platform and errors on same-level ties.
Findings beyond the spec: `.chezmoiignore` lines are globs, so paths with `* ? [ ] { } \` must be escaped (done); ignoring all files in a directory leaves the empty directory managed; an ambiguous variant aborts plain `chezmoi apply`, so apply must use `--keep-going` to block only that file.

## Test results

- Host (macOS arm64): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean; `cargo test`: 36 unit + cli 12 + git_wrapper 4 + isolation 5 + spike 12 + tui 10, all passing. `clone_over_https` passes with `DOTWARDEN_NET_TESTS=1` (skips itself otherwise). PTY smoke test OK.
- Isolation test proves `~/.config/chezmoi` and `~/.local/share/chezmoi` are neither read nor modified (byte/size/mtime snapshot, plus hostile `CHEZMOI_*` env); mutation check confirmed the tests fail when the explicit flags are removed.
- Non-blocking test: a fake git sleeps 3 s; keypresses are processed and the loop quits in <2.5 s while the status job is still in flight.
- Linux was never run (AGENTS.md: macOS only; the owner verifies Linux later). The earlier Docker harness was removed in the review round.

## Known gaps / deviations

- Linux (Ubuntu, Arch, CachyOS) behaviour is unverified by design; Linux-specific code (os-release detection) is covered by unit tests that run on macOS.
- Arch family: `ID=arch` has no `ID_LIKE`; I treat base IDs (arch, debian, …) as their own family, mapping `arch` → `arch-like`.
- Machine registration: duplicate names are refused unless `--adopt` (same platform/distribution) is passed; retired names are never reused. Spec does not say; this is my interpretation.
- `dw init` requires a global git identity and fails clearly otherwise.
- Chezmoi cache lives under `$XDG_CACHE_HOME/dotwarden/chezmoi` (spec lists no path).
- Pending count and vault state in status are stubs (`0`, `none`). Panels 3/4/5/Pending are placeholders. No delta rendering yet (delta is fetched and version-checked only).
- Not verified: macOS x86-64, Linux arm64 on Arch/CachyOS, the minimum glibc claim beyond the glibc 2.39 (Ubuntu) and 2.44 (Arch/CachyOS) images used, `.deb`/bundle packaging (not in T1).

## Review fixes (branch rebased onto `v2` with AGENTS.md and the macOS-only §13; no conflicts)

Test counts now: 53 unit + cli 17 + git_isolation 6 + git_wrapper 4 + isolation 4 + registry 3 + spike 20 + tui 12, all passing on macOS; fmt and clippy `-D warnings` clean; PTY smoke OK. Where noted, I confirmed by mutation that the new test fails without the fix.

1. **Git isolation** (`store.rs`, `toolchain.rs`): `GIT_CONFIG_NOSYSTEM=1` + `GIT_CONFIG_GLOBAL=/dev/null`; `user.name/email/signingkey`, `commit.gpgsign`, `gpg.format/program`, `gpg.{openpgp,x509,ssh}.program` are read from the user's global config (honouring the user's own `GIT_CONFIG_GLOBAL`) and forwarded as `GIT_CONFIG_COUNT/KEY/VALUE`. `tests/git_isolation.rs`: hostile global config (hooksPath hooks, fsmonitor, `url.insteadOf`, alias, `init.defaultBranch`) leaves no marker through init/status/doctor/clone, the rewrite does not break a clone, identity and signing still apply, system/env config (`GIT_CONFIG_SYSTEM`, `GIT_CONFIG_COUNT`, `GIT_CONFIG_PARAMETERS`, `GIT_DIR`) never apply, `GIT_SSH_COMMAND`/`SSH_AUTH_SOCK` reach ssh. Mutation-checked (4 of 6 fail without the fix).
2. **`.chezmoiignore` escaping**: verified against chezmoi that a leading `#` and `!` are syntax, whitespace is trimmed (also Unicode, e.g. NBSP), `#` mid-line is not a comment. Template escapes glob chars, leading `#`/`!`, and every separator as `[ ]`. `assign::set_assignment` refuses newline/CR/tab/control characters, absolute, `..`, empty components and bad targets with a usage error before writing. Tests for `#x`, `!x`, `a b `, ` lead`, NBSP, `[1]`, `*`, `?`, `{a,b}`, backslash. Mutation-checked.
3. **Spec §9**: `error.rs` (typed `DwError`, exit codes 1–7), JSON error objects on stdout with `--json` (including clap usage errors), `lock.rs` (flock via `File::try_lock`, waits up to 10 s, `DOTWARDEN_LOCK_WAIT_MS` for tests) taken by init/clone only, `--dry-run` for init and clone. Tests for each. Bad machine name and missing `--name` are now exit 2 (usage).
4. **`exec::run`** returns within a 300 ms grace after the child exits even if a grandchild holds the pipes (test with a backgrounded `sleep 8`; hangs without the fix).
5. **chezmoi subprocesses** get the bundled git first on `PATH` and the same git env (tested through a template calling `git --version` and `git config`).
6. **`toml_edit`** for registry and assignment edits (comments and order preserved, tested); atomic temp+rename writes for `chezmoi.toml`, `machine.toml`, data files and layout files. `status` writes nothing (the config sync was removed; git runs with `--no-optional-locks`); a test deletes the config and rewrites a tracked file, then snapshots (content + mtime) before/after; mutation-checked. `doctor` reports a missing/stale config instead of repairing it.
7. **Selection**: shared `dotwarden/select` helper used by `variant`; retired machines are refused by `targets`; unknown target kinds never match and `dw doctor` warns (new `warning` field); tests include "more specific match beats a lower-level tie".
8. Replaced the tautological test with `assigning_changes_the_data_file_and_the_managed_set_but_never_chezmoiignore` (behaviour + `git diff --name-only`); the system-config test now proves behaviour.
9. **Hide rules**: `/Library/` hidden with explicit `!/Library/Application Support/<app>/` un-hides (Code, Cursor, Zed, iTerm2, Rectangle). `HideRules` now treats literal `!` paths as reveal paths (they and everything below stay visible, folders leading to them are shown) because gitignore cannot re-include below an ignored folder. Dotwarden's own data/state/cache directories are hidden wherever XDG puts them (`always_hide`).
10. **Keys**: `g`/`G` are no longer bound (`G` is reserved for lazygit); `<`/`>` and Home/End jump to first/last; help updated; test asserts `G`/`g` do nothing.

Also: removed the Docker harness (`docker/`) to match AGENTS.md; spec §3 "Verified mechanics" updated (escaping, select helper, retired, git isolation, template code regenerated from `assets/`).

### Not done / caveats
- `~/Library` is still listed at the HOME root, only when one of the un-hidden app folders exists (needed to reach it); it then shows only the path to them. With no existing un-hidden path it is fully hidden.
- Retired machines make chezmoi fail loudly (rather than yield an empty target set); this is my reading of "filter retired machines out of targets".
- There is no `dw assign` command in T1: `assign::set_assignment` is the tested library entry point later tickets use.
- `doctor` cannot repair a missing generated config; only registration writes it.
- The lock is advisory (flock) and only init/clone take it; later mutating commands must call `Store::lock`.
- The `dry-run` of `clone` cannot check name conflicts with the remote registry (it does not contact the network).
- The 300 ms pipe grace means output a grandchild writes later is dropped (by design).
