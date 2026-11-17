# T17 — Sync and backup state

## Built
- `dw sync [-m MSG… | -e] [--yes] [--dry-run] [--remote NAME] [--json]` (`src/sync.rs`, `cmds::sync_cmd`).
  Step 1 `remote::pull` with the new `PullMode::FastForward` (diverged → exit 1 naming `dw pull`;
  a remote without the branch yet is nothing to pull). `sync::plan` (no writes) runs
  `apply_incoming`/`stage` as dry runs; `sync::run` does apply → conflict/locked-vault stop (5/4,
  nothing staged) → `changes::stage(None)` (skips secrets without a vault, clashes, refused
  templates) → `changes::commit` (generated `sync from <machine>: N changed, M new (…)`, `-m`, or
  `-e` with the generated text in the editor; new `CommitOpts::template`) → push when ahead.
  Push: humans with a terminal (after `Proceed? [Y/n]`) or `--yes` push; agents and no-terminal
  runs add a pending push item and exit 3. `--json` prints `{plan, done}` with `done.stop`
  (`kind`, `code`, `message`); exit = the stop's code, else 0.
- Stage and commit go through the existing journalled functions, so `dw undo` reverts the sync
  commit while it is on no remote; the activity log records `sync` like any mutation.
- Backup state (`src/backup.rs`): `remote::push` writes `last-push.json` (state dir) after each
  successful push, so every push path (push, pending apply, sync, clone, TUI) records it.
  `status` gains `backup {last_pushed, last_pushed_days, unbacked, unbacked_commits,
  waiting_days, warn_days, warning, summary}`; human status prints `Backup:`/`Warning:` lines.
- `src/prefs.rs`: minimal preferences reader (`Prefs::load`, `get`, `uint`,
  `backup_warn_days`, default 7); the vault unlock timeout now reads through it (same key).
- TUI: `S` (was unused) queues a background action that pulls and plans; the plan opens a
  `Dialog::Sync`; `y` runs it as another background action (stops show the details dialog).
  Status panel line `backup: …` (yellow when a warning) and a one-line banner `⚠ … — S sync`
  above the panels. Help: the `f p P z` row became `f p P S z`.
- Skill: `dw sync` for agents (pending push, exit 3/4/5, `skipped`, `backup` in status).

## Decisions
- The interactive/TUI plan is made after the fast-forward pull, so it lists exactly what will
  be applied and staged; `--dry-run` plans from the last fetch. Declining keeps the pull.
- Safe Incoming files are applied before a conflict/vault stop (the stop still precedes staging).
- `--remote` is both pull source and push target. Divergence is exit 1 (no spec code).
- "Oldest unbacked change" = this machine's last push (an upper bound), else the oldest commit
  no remote holds; unbacked commits = `upstream..HEAD`, or `HEAD --not --remotes` without one.
- Declining `Proceed?` exits 1 like a declined push. `--dry-run` always exits 0.
- Spec: §6 (backup age, `last-push.json`) and §6a (plan after pull, `--remote`, divergence exit,
  apply-before-stop) clarified; no other deviation.

## Tests
- `tests/sync.rs` (7, real git/chezmoi, bare remotes): two machines sync each other's edits
  (plan text, generated message, backup clean afterwards); conflict → exit 5, nothing staged,
  pull kept; secret in a tracked folder skipped, value absent from output and `git log -p --all`;
  agent sync → exit 3, local commit with `Agent:` trailer, pending push, then `dw undo` drops the
  commit with changes staged; `--dry-run` changes nothing; status backup fresh / warning via a
  10-day-old `last-push.json` / cleared by `backup_warn_days = 30`; TUI `S` → plan → `y` pushes.
- Unit: generated message counts and path list.
- Hand-broken: conflict stop removed, plaintext allowed in staging, agent push not gated — each
  failed its test; restored.
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, full `cargo test` green.

## Gaps
- The `Proceed? [Y/n]` prompt and `-e` need a real terminal; not covered by tests.
- The TUI runs the plan it showed; a HOME change between dialog and `y` is synced as it is then.
- `--dry-run` cannot show files the pending pull would make Incoming or Conflict (it says so).
- `dw doctor` does not report invalid preference values yet (T14).
