> Historical Go baseline. The Rust rewrite supersedes this implementation; see [Rust spec](rust-rewrite-spec.md) and [current verification](rust-acceptance.md).

# Native whole-spec acceptance (#19)

Local gate date: **2026-10-14**, Darwin ARM64, Go **1.27.2**. Application source
is ticket branch `implement/spec10-19-native-acceptance`, commits `039ac29` and
`837a8cb`, based on integration `60a708f`. The integration merge was checked and
was already up to date. These are local acceptance version labels, not tags or
published releases. The existing v-tag publisher is unchanged; no CI was added.

**Final local result: PASS** for `go test ./...`, `go test -race ./...`,
`go vet ./...`, both Linux cross-builds, all rc.5/rc.6 package verification, and
actual extracted Darwin full acceptance plus rc.5 → rc.6 upgrade/conflict-safe
rollback. Independent repeat rc.5 build checksums are identical. Linux runtime
execution remains **waived, not passed**.

## What the gate executes

`cmd/dot-accept` verifies the trusted archive checksum before extraction, then
executes the **extracted dot** outside the checkout. Ordinary journeys use a new
HOME, private state/config, a local Git remote and PATH containing **only system
Git**; chezmoi is the private verified bundle. No interpreter, shell, gum, gh,
compiler, system chezmoi, checkout fallback or second application is available.

| Boundary | Actual evidence |
| --- | --- |
| Complete keyboard new-user journey | Bare app, local init/remote-add, real terminal paste, edited Huh identity, verified dependencies, tracker fallback inputs, Cancel-default review, confirmed connection, Done and Finish |
| Complete existing-user/machine-2 journey | Real system Git clone through its local HTTPS transport rewrite, distinct edited identity, Tracker/Done, explicit pull, machine JSON, keyboard-selected Adopt |
| Continue / Replace | Existing real checkout, cancel-first path-naming second review, one timestamped full backup, preserved Git, committed-but-dirty manifest, binary untracked bytes and 0751 mode |
| Real keyboard configuration | `M` prepares valid native logical metadata, exact Track plan/preview, Cancel-default final confirmation, explicit apply; clone and exact keyboard Adopt preserve NUL/non-UTF8 bytes and Git executable mode |
| Assignment/local policy | Real packaged Track/apply; all/include/exclude eligibility, inapplicable explicit-id rejection, composed profiles, persisted deselection across profile switch, local customs, machine-1 state unchanged, future profile additions excluded from Update |
| Independent gates | Plan requires Unsafe, desktop and existing-backup acknowledgement; omit each separately and check live/backup bytes unchanged, then exact consent preserves reviewed bytes/modes and original backup |
| Freshness/concurrency | Human target edit rejects a saved plan; two actual processes consume a changing approval at most once; source/state/backup freshness also covered by native integration tests |
| Interrupted configuration | Durable partial-action journal fixture (not a mocked success): pending recovery blocks apply, conflicting human edit is refused, exact recovery restores original target/backup bytes and modes |
| Native terminal boundaries | Compiled isolated non-installing brew child receives real terminal streams and exact install/environment policy; Space invokes nothing, zero manager exit cannot claim readiness, actual Ctrl+C returns 130; real Git clone is interrupted while its isolated SSH child blocks |
| Fail closed / offline | Malformed shared/private state, invalid assignment shapes and symlink enrollment refused; credential URLs produce secret-safe errors before writes; unsupported helper template cannot execute; init-check/contract/status/diff/scope succeed with unusable remote |
| Public distribution | Full help before setup, trusted-checksum install, foreign-manager ownership refusal, initial-promotion rollback, real next-version upgrade/rollback, pending/conflicting promotion refusal, uninstall retains private approvals and immutable release data |

Manager/SSH fixtures are compiled **by maintainer tooling**, never by dot. They
record arguments and exercise streams but install nothing. The SSH child cannot
contact a remote. Recovery interruption fixtures model durable on-disk states;
they do not claim that every possible instruction boundary was signal-injected.
Normal app journeys remain Git-only PATH. Filesystem policy preserves actual
source modes; Git itself transports only regular/executable mode, not arbitrary
Unix permission bits.

## Fixed integration defects and regressions

- New entry/request assignment lists could bypass the nonempty-object policy.
  Legacy list compatibility is now restricted to old logical metadata; strict
  duplicate/unknown-field validation remains intact.
- Profile switching discarded machine-local deselections. They now survive;
  explicit reselection clears an exclusion.
- The native Git adapter forcibly disabled system config. It now honors inherited
  Git configuration; a real `safe.directory` system-config regression proves it.
- Default logical-editor requests had an invalid id and list-shaped assignment;
  empty metadata maps also changed the exact plan on JSON roundtrip. The editor
  generates valid stable ids/objects and Plan owns canonical serialized requests.
- Returning/intent/package operations ignored app cancellation, and raw Ctrl+C
  exited successfully. All share the app context; actual interruption exits 130.
  Transition from setup Done retains the live context for returning operations.
- Bubbles/Huh clipboard shortcuts could invoke platform helpers. Dot handles
  terminal bracketed paste instead, with a real packaged Ctrl+V guidance check.

Huh **v2.0.3** is genuinely embedded in the machine page via Init/Update/View,
not an unused dependency or separate Run/prompt program. Official upstream README
and the installed version's `form.go`, input API and completion behavior were
consulted: <https://github.com/charmbracelet/huh>,
<https://github.com/charmbracelet/huh/blob/v2.0.3/form.go>.
Context7 tool discovery exposed no resolve/query tools, so official source was
the documentation fallback. The TUI detector reported no findings; real PTY
transcripts include the rendered Huh field and submitted edited identity.

## Exact final artifacts and retained fixtures

Every relative path below is beneath the approved external base:
`/private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/`.

- `dots18-upstream/`: independently verified pinned upstream offline cache;
  provenance/lock hashes remain in [native distribution](native-distribution.md).
- `dots19-final-v2-dist/`: fresh final `0.10.19-rc.5` three-platform archives and SHA256SUMS.
- `dots19-final-v2-update-dist/`: fresh real `0.10.19-rc.6` upgrade archives and SHA256SUMS.
- `dots19-final-v2-accept/`: final extracted Darwin artifact, Git-only PATH,
  isolated HOMEs/source/state, `plans/*.json`, `help-evidence.json`, and terminal
  transcripts (`keyboard-macos_one`, `keyboard-macos_two`, `keyboard-continue`,
  `keyboard-replace`, `keyboard-track`, `keyboard-adopt`, `keyboard-interrupt`,
  `keyboard-manager-{skip,zero,interrupt}`, `keyboard-source-interrupt`).
- `dots19-final-linux-amd64-inspected/`, `dots19-final-linux-arm64-inspected/`:
  latest verified Linux extractions; static inspection only, not execution.
- `dots19-repro-v2-dist/`: independently rebuilt rc.5; `cmp` of SHA256SUMS
  against final-v2-dist exited 0 for all three platform archive hashes.
- `dots19-final-v2-accept/evidence/`: exact captured outputs
  `packaged-acceptance.stdout`, `go-test.stdout`, `go-test-race.stdout`,
  `go-vet.stdout`, `build-rc5.stdout`, `build-rc6.stdout`. Vet's empty output is
  expected; its command exited 0.
- Earlier `dots19-first-*`, `dots19-second-*` and `dots19-final-dist/` rc.3/rc.4
  artifacts are diagnosis/pre-final evidence only.

Final rc.5 archive SHA256 values:

```text
56568ad263e63f1e49e3122ae0665ce85978afc8a9ceb3123f61d1dd19951b96  dot-0.10.19-rc.5-darwin-arm64.tar.gz
9dbee3549dbcbabb31ddbe5da204ed8f0ef94eec24a24115540bfd609ba1be6f  dot-0.10.19-rc.5-linux-amd64.tar.gz
214f0ea3c5e3b940168fc63e5b6f5c5c6b757250b8e4477dcfe43a1892d638ea  dot-0.10.19-rc.5-linux-arm64.tar.gz
```

Final rc.6 real-upgrade archive SHA256 values:

```text
24b1e9aab28ac6798b699c8a7d2665bf7d08751214d2ee0f7ff3a45c638ccdcb  dot-0.10.19-rc.6-darwin-arm64.tar.gz
ff146cf8fcf1a07335e162b5361859ac1f7ad7212bf0c2938efae4cfdcea43ab  dot-0.10.19-rc.6-linux-amd64.tar.gz
a1096af99e8749d6901723f5c4a80c7d7a68de108e8694ee1b181522635c0ed0  dot-0.10.19-rc.6-linux-arm64.tar.gz
```

## Reproduce the local gate

Run from a checkout with the ticket commits, choosing fresh output/extraction
directories. Commands build no tags and perform no host installs:

```text
go test ./...
go test -race ./...
go vet ./...
env GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build ./...
env GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build ./...
go run ./cmd/dot-release build --version 0.10.19-rc.5 --out /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-final-v2-dist --deps-dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-upstream
go run ./cmd/dot-release build --version 0.10.19-rc.6 --out /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-final-v2-update-dist --deps-dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-upstream
go run ./cmd/dot-accept --archive /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-final-v2-dist/dot-0.10.19-rc.5-darwin-arm64.tar.gz --sha256 56568ad263e63f1e49e3122ae0665ce85978afc8a9ceb3123f61d1dd19951b96 --dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-final-v2-accept --update-archive /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-final-v2-update-dist/dot-0.10.19-rc.6-darwin-arm64.tar.gz --update-sha256 24b1e9aab28ac6798b699c8a7d2665bf7d08751214d2ee0f7ff3a45c638ccdcb
```

Each build statically verifies all three packages, exact inventory, metadata,
architecture, pinned dependency/license, hashes and modes. Dot-accept's
`--update-archive`/`--update-sha256` requires an actual different version; it cannot
pretend a same-version update proves upgrade/rollback.

## Limits and release ownership

**Linux runtime execution was explicitly owner-waived.** Both Linux platforms
are cross-built and statically verified, not executed; Docker was not started.
amd64 chezmoi requires glibc (`/lib64/ld-linux-x86-64.so.2`, `libc.so.6`); arm64's
upstream executable is static. Minimum glibc and musl compatibility are not proved.
Real hosting-service authentication, live network outages, privileged pacman and
host Homebrew installation are not exercised. Native isolated fixtures establish
terminal/consent policy, not successful host provisioning.

The local gate is review evidence, not authorization to tag/push, close #10/#19,
or mark a PR ready. The orchestrator owns final review and release/issue decisions.
