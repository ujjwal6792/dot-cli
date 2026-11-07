# T13 — Recovery: undo and archive

## Built
- `src/undo.rs`: undo journal `undo.json` (20 entries) + `undo/` copies of chezmoi's
  state. Entry = `(HEAD, index tree)` before/after, or a snapshot id for a discard.
  Hooks: `changes::{stage,unstage,commit}`, `hunks::apply`, `track::apply_review`
  (every applied pending item: track, untrack, assign, variants, values...) and the
  discard arm of `apply_items`.
- `dw undo [--dry-run]`, TUI `z` (+ help line).
- `src/archive.rs`: `dw archive export|import` (`--dry-run`, `--force`), age scrypt
  seal under the vault passphrase; header carries machine name and remote/upstream config.
- `repo::check_repository` extracted from `clone` and shared with import.
- Spec §11 updated.

## Decisions
- Undo applies only if the repository still equals the entry's "after" state (exit 7
  otherwise), so a pull/revert/hand edit can never be clobbered; those actions are not
  journalled, so they make older entries unreachable until a new action is recorded.
- Commit undo uses `reset --soft` and reuses `ensure_amendable` as the "unpushed" test.
- Discard undo is `snapshot restore`, which snapshots current HOME first.
- Export requires the passphrase to be the vault's; import carries no privacy
  confirmations and does not need the vault to be unlocked.
- Recording is best effort and never fails the action it follows.

## Tests
- tests/recovery.rs: ordered undo of commit/track/stage/discard (HEAD, index tree and
  HOME verified), pushed-commit refusal and stale refusal, empty journal, archive
  round trip into a fresh HOME (wrong passphrase, sealed content, branches, remote
  config, machine name, `apply --all`, vault unlock). tests/tui_changes.rs: `z`.
- Hand-break: disabling the stale guard failed the stale test.

## Gaps
- Pull, revert, branch/stash, restore --at and resolve are not undoable by `z`.
- Assign is covered by the shared pending-apply path, not a dedicated test.
- Pending list and uncommitted work are not in the archive.
- Not pushed; no GitHub changes.
