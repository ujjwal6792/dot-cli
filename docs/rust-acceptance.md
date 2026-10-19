# Rust rewrite verification

Date: 2026-10-19. Local implementation version: `0.2.0-dev.1`.
This records evidence for the uncommitted Rust rewrite, not a published release.
The accepted contracts are in [the spec](rust-rewrite-spec.md); historical bug IDs
remain in [the bug register](tui-redesign-bugs.md).

## Environment and preservation

- Native macOS ARM64; Rust/Cargo 1.93.1; Ratatui 0.30.2; chezmoi 2.73.1; system Git.
- The former Go implementation was archived before removal at
  `/private/tmp/dot-go-before-rust-20261019-223143.tar.gz`.
- Git history and design evidence remain. Go application/build/test source was
  replaced by the Cargo package, Rust modules and native integration tests.
- Mutation tests use private temporary HOME, source, native config and local state
  directories. Remote tests use local bare Git repositories. Verification did not
  migrate the user's live catalog, apply their HOME configs, install packages or
  activate a background service.

## Executed checks

| Check | Result |
|---|---|
| `cargo fmt --check` | Pass |
| `cargo clippy --locked --all-targets -- -D warnings` | Pass |
| `cargo test --locked` | 27 passed: 10 unit/UI/process, 17 integration |
| `cargo build --locked --release` | Pass; `dot 0.2.0-dev.1` |
| Agent skill validator | Pass for `skills/dot/SKILL.md` |

Native integration tests cover capture, staging and committing with unrelated
index entries; restore preserving unrelated enabled paths; empty selection;
stale HOME/config/plan rejection; template capture refusal; folder aggregation and
untracked visibility; safe recovery with intervening-edit refusal; path traversal
and symlink-parent rejection; forget preserving HOME; incremental scope; report
publication without changing the config branch/index; strict hook rejection before
execution; source switch preservation/reset; shared → OS → hostname values rendered
by chezmoi; real fast-forward synchronization restricted to enabled files; legacy
migration with held entries; exact partial outcomes; and cross-state source locking.

UI tests feed actual search key events, preserve independent Files/Git selection,
exercise footer pointer dispatch, and render at 120×35, 80×24, 45×14 and 20×8.
The synthetic Ratatui buffer was also rendered to an image and visually inspected.
Process tests cover timeout termination, inherited output pipes after the parent
exits, and simultaneous large stdin/stdout without pipe deadlock.

The real PTY smoke script initializes/captures an isolated fixture, launches the
application, types `q2Wconfig` into search, resizes to 45×14, opens Actions, exits and
checks terminal restoration. On macOS, a revoked terminal slave is handled by
checking the emitted alternate-screen restoration sequence.

## Bug coverage

| IDs | Implementation and evidence |
|---|---|
| BUG-01 | Restore and local automatic scope are separate; native integration preserves unrelated scope. |
| BUG-02 | Files/Git selections are independent; operation plans receive fresh identities; UI navigation regression test. |
| BUG-03 | Forward/reverse previews use native chezmoi; capture refuses to flatten templates. Old custom diff engine removed. |
| BUG-04 | Focused text handles character keys first; actual-key UI and PTY regressions. |
| BUG-05 | Operation-specific progress and exact succeeded/failed/not-attempted outcome; real partial-apply fixture. |
| BUG-06 | Mutation completion refreshes the snapshot; plans validate source/config/Git fingerprints; real pull fixture. |
| BUG-07 | Global Actions includes Packages and settings; reachable after narrow resize in PTY. Actual package installation remains untested. |
| BUG-08 | Footer hit targets share action dispatch; pointer regression at multiple sizes. |
| UX-01/02 | Native capture, actual index, explicit commit/push and separate HOME/Git state are implemented; Git/native fixtures exercise core boundaries. |
| UX-03 | Source switching archives before connecting and resets local trust/scope; native integration test. |
| UX-04 | Persistent contextual workspace, directory rows, large detail area, full-screen forms and narrow focus mode; buffer and PTY checks. |

This is targeted regression coverage, not an exhaustive proof of every combination
of native chezmoi attributes, templates, terminal emulators or Git configurations.

## Distribution and remaining host validation

`scripts/package.py` verifies the pinned official chezmoi archive, binary and license
before packaging. The bundle contains the Rust executable, private chezmoi binary,
license and per-file hash manifest. Bundle verification is recorded below.

Verified local artifact: `dist/dot-0.2.0-dev.1-darwin-arm64.tar.gz`.

```text
SHA256 d95bdd528fbcfd5761899b5564e7c164349e2588886a9f412cc477ae9ad2770a
```

The refreshed exit-fix bundle's sidecar checksum and every manifest file hash
matched after extraction to `/private/tmp/dot-exit-fix-gcu6576b`. It contains
`dot 0.2.0-dev.1` and chezmoi `v2.73.1`. The missing-source PTY regression passed
against the extracted bundle: Esc/Enter stays dismissed after the refresh deadline,
q/Ctrl+C exits, and terminal restoration is emitted. The ordinary init/capture,
search and resize PTY journey also passed against the updated release executable.

The missing-source regression first failed at `dismissal immediately retried the
failed refresh`, and its separate error-screen q test failed before the fix.
Failed inspections now pause automatic retries until explicit refresh/setup;
inspection jobs no longer block exit. Active mutations retain their exit guard.

The tag-triggered release matrix configures native macOS ARM64, Linux x86-64 and
Linux ARM64 builds, tests and PTY checks. That workflow has not been run by this
local rewrite. Linux runtime behavior, actual launchd/systemd activation, package
manager authentication and real network/offline recovery need host acceptance.
No tag, GitHub release, remote push or machine-wide installation was performed.

## Startup listing follow-up — 2026-10-20

The initial HOME listing now loads independently of the native snapshot. Two new
regressions reproduce an empty startup list and files appearing only after Left
when native inspection fails. Both failed before the fix; all 9 UI tests now pass,
along with formatting, Clippy and debug/release builds. The missing-source PTY
script also verifies a fixture file is visible after dismissing the error, without
any navigation. This follow-up rebuilds the local executables; the bundle checksum
above identifies the earlier packaged exit fix.

## Section tabs follow-up — 2026-10-20

Every workspace section now has visible header tabs. `[` and `]` cycle and wrap
within the focused section; mouse labels use the same transition. Other sections
retain their tabs. Borders are subdued and the focused active tab carries the
accent. Narrow layouts keep the selected tab visible. Git commit history does not
accept file stage, selection or discard shortcuts.

All 12 UI tests pass, including independent section state, wrapping, text-input
ownership, mouse hitbox alignment and narrow tab visibility. Formatting, Clippy,
debug/release builds and the real PTY smoke journey pass. The Ratatui buffer was
rendered and visually reviewed. The local binaries include these changes; the
previous bundle checksum does not describe this follow-up.

Strict automation intentionally accepts only a conservative regular-file subset;
dynamic source needs explicit local trust. Complex legacy entries remain held for
manual translation. Arbitrary templates have no automatic inverse mapping.
Advanced Git history operations and hunk staging remain outside this version.
Recovery restores supported files; it cannot undo arbitrary script or remote effects.
