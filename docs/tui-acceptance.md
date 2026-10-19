> Historical Go baseline. The Rust rewrite supersedes this implementation; see [Rust spec](rust-rewrite-spec.md) and [current verification](rust-acceptance.md).

# Terminal workspace acceptance — 2026-10-16

The extracted darwin-arm64 release passed the packaged end-to-end gate, including
an actual upgrade between different versions and rollback. All effects used
isolated HOME, source, state, installation and local Git remote directories under
`/private/tmp/dots-cli-e2e-20261016`; the host configuration was not enrolled.
The app ran outside this checkout with only system Git on PATH and its verified,
private chezmoi dependency. Release versions below are local test labels.

## Verified behavior

- Keyboard setup (new and clone), identity, tracker consent, Continue and Replace.
  Replacement preserves the original Git checkout, dirty bytes and executable modes.
- All six tabs in both directions using terminal Tab and Shift+Tab bytes, including
  wraparound. Settings search consumes typed `q`; Ctrl+N/Enter activates Verify;
  tracker verification succeeds without installation or application.
- A 10,000-file browsing fixture: 1,000 immediate files and 9,000 in a collapsed
  child. Open Track, expand the parent, navigate to its final file with End, return
  with Home, then quit. The final run completed that journey in 309.164208 ms,
  excluding fixture creation. This is one local smoke measurement, not a benchmark.
  Browsing leaves adoption scope empty and creates no tracked source files.
- Track and Adopt through exact plan review, Cancel-default final confirmation,
  cancellation and explicit application. Adoption preserves binary bytes and 0755.
- Scripted status/diff/scope JSON, machine assignments, profile unions, local
  overrides, stop tracking (preserving target/source bytes, mode and local approval),
  separate unsafe/desktop/backup acknowledgments, stale
  plans, concurrent application and interrupted-operation recovery.
- Unknown fields, destination collisions and unsafe paths fail closed. Pending
  recovery blocks current Apply plans and remote changes. Recovery refuses human
  edits; dirty pull refuses changes; inspection works with an unusable remote.
- Help, health, version, Ctrl+C (130), dependency skip/missing/interrupt, and
  interrupted clone. Manager fixtures never install real host packages.
- Bad archive checksums, install, same-version update, uninstall, initial-install
  recovery, different-version upgrade, foreign-pointer refusal and rollback.

The gate exposed and fixed Git stderr contaminating parsed stdout and hidden
replacement choices. Regression checks also cover concurrent repository-cache
access and rejection after repository removal. The terminal runner now follows
the six-tab workspace and accounts for ANSI styling and incremental repainting.
Its pending-recovery assertion uses a current plan so a stale-plan refusal cannot
accidentally satisfy the recovery check.

## Commands and evidence

All commands ran with `GOCACHE=/tmp/dots-cli-go-cache`:

```sh
go test ./...
go test -race ./...
go vet ./...
go build -buildvcs=false -o dot ./cmd/dot
```

Both release builds used `go run ./cmd/dot-release build` with the pinned upstream
asset cache, versions `0.10.20-rc.4` and `0.10.20-rc.5`, and output directories
`release-final` and `release-final-update` beneath the fixture root. Each build
cross-built and statically verified darwin-arm64, linux-amd64 and linux-arm64.
The upstream cache used was
`/private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-upstream`.

```sh
go run ./cmd/dot-accept \
  --archive /private/tmp/dots-cli-e2e-20261016/release-final/dot-0.10.20-rc.4-darwin-arm64.tar.gz \
  --sha256 f586e4c0cd12cbb5d12e35e1bb85d81cfe67531046b2b8c316ebcc14ad3eb9cf \
  --dir /private/tmp/dots-cli-e2e-20261016/accept-full \
  --update-archive /private/tmp/dots-cli-e2e-20261016/release-final-update/dot-0.10.20-rc.5-darwin-arm64.tar.gz \
  --update-sha256 c7ba216798eb290c09da6f5972a992bbdf61ea9a32ca4b0298c31e1ec060e5bd
```

Use a new `--dir` when repeating the gate. Final stdout is retained in
`evidence/accept-full.log`; unit/race/vet logs are `evidence/go-test-final.log`,
`evidence/go-test-race-final.log`, and `evidence/go-vet-final.log`.
Terminal transcripts, plans, help JSON and timing remain in `accept-full`.
Earlier `accept-*` directories hold diagnosis or narrower passing runs;
`accept-full` is the final gate, including Stop-Tracking preservation checks.

```text
PASS: real extracted darwin-arm64 full keyboard setup/Continue/Replace/Track/Adopt, Tab/Shift+Tab/Settings/10,000-file browsing, Huh identity, manager/source interrupts, assignments/profile unions/local overrides/gates, machine JSON, byte/mode/stale/concurrent/recovery/offline safety, private install/update/recovery/uninstall
PASS: real darwin-arm64 upgrade 0.10.20-rc.4 → 0.10.20-rc.5 and conflict-safe rollback to 0.10.20-rc.4
```

Final archive SHA256 values:

```text
f586e4c0cd12cbb5d12e35e1bb85d81cfe67531046b2b8c316ebcc14ad3eb9cf  dot-0.10.20-rc.4-darwin-arm64.tar.gz
384f83d757673e6bd26005c6c402215b9c5973e91ae19b3e7bba94fc25d7ec96  dot-0.10.20-rc.4-linux-amd64.tar.gz
c3fcd0e2e435625ecb26a9bb6ec888c9747de781b5c23a329f70fbd359d07f29  dot-0.10.20-rc.4-linux-arm64.tar.gz
c7ba216798eb290c09da6f5972a992bbdf61ea9a32ca4b0298c31e1ec060e5bd  dot-0.10.20-rc.5-darwin-arm64.tar.gz
7d9a6aaab0e0f9e41b0a78ceb1ae04ee4c1cde2e053a5ec75430d730581f55e9  dot-0.10.20-rc.5-linux-amd64.tar.gz
b3cbb9a3ae6c040cf67cca6060fd4998de8fb836b8e30db4466706c3c3f71822  dot-0.10.20-rc.5-linux-arm64.tar.gz
```

## Limits

Linux packages were cross-built and statically verified, not executed. Real
hosting authentication, privileged package provisioning, host Homebrew/pacman
installation, live network outage behavior and minimum glibc/musl compatibility
were not tested. This gate does not publish a release or push a Git commit.
