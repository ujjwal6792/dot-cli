# T1 report — Walking skeleton (branch `t1-sonnet`)

## What was built

- Cargo project (edition 2024): `dotwarden` and `dw` binaries (second `[[bin]]`) sharing a lib crate.
- **Toolchain** (`src/toolchain.rs`): resolves `$DOTWARDEN_LIBEXEC`, then `<exe>/../libexec`, then (debug builds) the checkout's git-ignored `libexec/`. Pins live in `toolchain/pins.env` (chezmoi 2.73.3, dugite-native v2.56.0-2 / git 2.56.0, delta 0.20.1; checksums cross-checked against upstream checksum files and release digests). `scripts/fetch-tools` downloads and SHA-256-verifies them for darwin-arm64, linux-arm64, linux-amd64. Dugite git env (`GIT_EXEC_PATH`, `GIT_TEMPLATE_DIR`) is set by the wrapper.
- **Isolated store** (`src/store.rs`): every chezmoi call passes `--config/--source/--persistent-state/--cache/--destination` and has `CHEZMOI_*` stripped. Git runs with `GIT_CONFIG_NOSYSTEM=1`, redirecting `GIT_*` variables removed, identity from the global gitconfig. XDG dirs respected.
- **`dw init [--name] [--branch]`, `dw clone <url> [--name] [--adopt]`**: spec §3 layout, `.chezmoiroot`=`home`, fixed `.chezmoiignore`, helper templates, default `hide`, `dotwarden.toml`. Machine registration (name from `--name` or prompt on a TTY; platform/distribution/families from os-release or macOS), registry entry in `home/.chezmoidata/dotwarden.toml`, local `machine.toml`, generated `chezmoi.toml`. Failures clean up the half-created repo.
- **`dw status --json`, `dw doctor`**, every flag/argument has help text (unit-tested).
- **TUI** (`src/tui/`): panels 1–5, main view, Pending pane, Files HOME browser (hide rules, `.` toggle, j/k/h/l/arrows), `?` help, `q`, resize, single-panel mode below 60×16. All I/O on a job runner (supersede-and-cancel by key); the event loop only waits on one channel.
- **Docker harness** (`docker/run.sh ubuntu|arch|cachyos`), plus `scripts/pty-smoke.py` (real binary on a PTY).

## How to run

```
scripts/fetch-tools            # into ./libexec (git-ignored)
cargo build
export HOME=$(mktemp -d) ; git config --global user.name X ; git config --global user.email x@y   # never use the real HOME
target/debug/dw init --name mybox ; target/debug/dw status --json ; target/debug/dw doctor ; target/debug/dw
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
docker/run.sh ubuntu | arch | cachyos
```

## Spike findings

Recorded in `docs/spec.md` §3 "Verified mechanics" with exact template code; each proven in `tests/spike.rs` and `tests/isolation.rs`. Highlights: `.chezmoiroot` works with the repo root as `--source`; leading-dot entries under `home/` are never targets; `include` paths are relative to `home/`; assignments (union includes, excludes win, four target kinds) evaluate in the fixed `.chezmoiignore` from registry data plus only the registry name in generated config; variant helper picks machine > distribution > family > platform and errors on same-level ties.
Findings beyond the spec: `.chezmoiignore` lines are globs, so paths with `* ? [ ] { } \` must be escaped (done); ignoring all files in a directory leaves the empty directory managed; an ambiguous variant aborts plain `chezmoi apply`, so apply must use `--keep-going` to block only that file.

## Test results

- Host (macOS arm64): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean; `cargo test`: 36 unit + cli 12 + git_wrapper 4 + isolation 5 + spike 12 + tui 10, all passing. `clone_over_https` passes with `DOTWARDEN_NET_TESTS=1` (skips itself otherwise). PTY smoke test OK.
- Isolation test proves `~/.config/chezmoi` and `~/.local/share/chezmoi` are neither read nor modified (byte/size/mtime snapshot, plus hostile `CHEZMOI_*` env); mutation check confirmed the tests fail when the explicit flags are removed.
- Non-blocking test: a fake git sleeps 3 s; keypresses are processed and the loop quits in <2.5 s while the status job is still in flight.
- Docker / Linux: **no Linux runs are part of the final results, by owner decision** (to be verified on real Linux machines later). The harness (`docker/`) is committed but was not executed for the final state. Earlier, before that decision, I had run it: Ubuntu 24.04 arm64 passed fully (fmt, clippy, all tests, https clone, PTY smoke) and Arch (amd64 under QEMU) passed all test binaries, both on an earlier revision of the harness; the CachyOS amd64 container hung under QEMU and was stopped. Treat these as unverified for the final commit.

## Known gaps / deviations

- Linux (Ubuntu, Arch, CachyOS) verification of the final commit was not performed; the Docker harness is untested in its latest form (QEMU workarounds added after the Ubuntu run).
- Arch and CachyOS images are x86-64 only (no arm64 manifest, verified by `docker pull`), so they run emulated amd64. Consequences: the linux-amd64 pins are exercised, but not linux-arm64 on Arch/CachyOS; pacman needed `DisableSandbox` (seccomp fails under QEMU); cargo deadlocked once under QEMU, so the harness limits jobs and retries; ratatui's terminal init fails with ENOSYS on a PTY under QEMU, so the PTY smoke test is skipped when emulated (covered natively on Ubuntu and macOS).
- Arch family: `ID=arch` has no `ID_LIKE`; I treat base IDs (arch, debian, …) as their own family, mapping `arch` → `arch-like`.
- Machine registration: duplicate names are refused unless `--adopt` (same platform/distribution) is passed; retired names are never reused. Spec does not say; this is my interpretation.
- `dw init` requires a global git identity and fails clearly otherwise.
- Chezmoi cache lives under `$XDG_CACHE_HOME/dotwarden/chezmoi` (spec lists no path).
- Pending count and vault state in status are stubs (`0`, `none`). Panels 3/4/5/Pending are placeholders. No delta rendering yet (delta is fetched and version-checked only).
- Not verified: macOS x86-64, Linux arm64 on Arch/CachyOS, the minimum glibc claim beyond the glibc 2.39 (Ubuntu) and 2.44 (Arch/CachyOS) images used, `.deb`/bundle packaging (not in T1).
