# T24 report: dogfooding round 1 and backup-workflow gaps

## Built

- **Release build from the checkout**: `target/release/dw` uses the checkout's
  `libexec/` when the binary lives inside the checkout (`toolchain.rs`).
- **Hidden paths**: `HideRules::track_refusal` names the rule and the
  `dw hide add '!/…'` command for exactly that path; used by track and import.
  Dotwarden's own directories keep "see `dw hide`".
- **Unstorable modes**: `classify` ignores a target whose HOME file has the
  same content and the same executable/private/read-only attributes (0744 vs
  0755). Dropping the exec bit is still Local.
- **XDG moved by a wrapper**: `Dirs::repo_elsewhere`; `status`, commands that
  need a repository and `doctor` name `~/.local/share/dotwarden/repo` and the
  `env -u` workaround; `init`/`clone`/archive import refuse a second repository.
- **Remote add clears its pending request**: `remote::forget_added` (CLI and
  TUI) removes pending `remote_add` items with the same name and URL.
- **TUI apply prompt** runs a dry-run apply in a job and lists replaced
  existing files (up to 8, then "… and N more").
- **Sync re-plan**: `sync::changed_since` re-makes the plan after `y` (no new
  pull); a different plan is shown and asked again (terminal loop, TUI dialog).
- **Import grouping**: `track::any_file` stops at the first unlisted file.
- **Dev builds**: scrypt, salsa20, pbkdf2, sha2 at opt-level 3; `vault init`
  went from 52 s to 1 s in debug, which speeds every vault test.
- Merged earlier: TUI remote add asks for the name, then the URL.

## Decisions

- Mode: treated as no change rather than chmodding HOME at track time, so
  tracking never touches HOME; chezmoi still writes 0755 when the file next
  arrives as Incoming for another reason.
- XDG: refuse instead of silently using the default location; an explicit XDG
  setting is still honoured when no default repository exists.
- Re-plan compares the rendered plan lines, i.e. exactly what was shown.

Spec: clarifications in §3, §5, §6, §6a; no deviations.

## Tests

416 passed, 0 failed before two clippy-only rewrites; the affected files
(sync, import, track, changes) re-run green. fmt and clippy clean.

New or extended: unstorable mode vs exec-bit drop (`changes`); suggested
hide rule really un-hides (`hide` unit); wrapper XDG for `status` and `init`
(`isolation`); stale remote request removed (`remote`); TUI apply prompt lists
replaced files; TUI sync re-plan; `Proceed?` n/Enter and `-e` on a real pty
(`sync`); import with an unlocked vault keeps the token out of the repo.
Broken by hand: the mode fix (test fails without it).

## Gaps

- HTTPS auth-failure hint: still unit-only; the hint needs a known host, which
  a test cannot reach offline. The success path (gh helper, real push) was
  verified while dogfooding.
- The terminal re-plan loop is not driven by a test (needs a HOME change while
  the prompt waits); the TUI path and `changed_since` are tested.
- Grouping speed: 50,000 unlisted files took ~0.5 s before and after; the early
  stop only bounds much larger trees.
- `dw doctor` preference validation stays with T14.
