# T3 report (branch `t3`)

## Part A: change model, stage, unstage, commit, diff

### Built

- `src/changes.rs`: one classifier (`scan`/`classify`) returning Local, New, Staged, Incoming, Conflict for files that apply to this machine (the set comes from `chezmoi dump`, so files assigned away never appear). Plus `stage`, `unstage`, `commit`, `diff`.
- CLI: `dw stage PATH…|--all`, `dw unstage PATH…|--all`, `dw commit [-m …]… [--amend]`, `dw diff [PATH] [--no-pager]`; `--json` everywhere, `--dry-run` on mutations. `dw status` now lists changes grouped by state (`changes` in JSON).
- Deps added: `sha2` (content hashes), `similar` (unified diff).

### Decisions (please check)

1. **Classification.** `chezmoi status` columns decide whether HOME differs from the target (col 2) and from the last-written record (col 1). Col 2 only: Incoming. Both: Local if the last-written hash equals the target hash, else Conflict. No last-written record: Incoming if gone from HOME, else Conflict. Deleted-in-HOME is Local (`deleted`); mode and symlink changes are Local.
2. **Staged** = git index vs HEAD, decoded to HOME paths. Staged repository files that are not HOME files (assignments, hide rules, folder records) show as `repo:` entries. A file edited after staging is both Staged and Local.
3. **Last-written state.** `chezmoi add` records HOME content as last written, which is why a committed file shows nothing. Unstage therefore writes the committed hash back (`chezmoi state set`), or deletes the record for a file that was new; otherwise the file would show as Incoming.
4. **Stage** runs `chezmoi add` then `git add` of exactly the changed repo paths; refuses if the repo has unstaged edits; all-or-nothing (index tree and chezmoi state are restored on failure). Refused when named explicitly: Incoming (exit 1), Conflict (exit 5), deleted-in-HOME (use `dw untrack`), templates. With `--all` or a folder argument these are skipped and listed.
5. **Unstage** = `git reset HEAD` + `git checkout` of the file's source paths (handles `executable_` renames); a newly added source file is removed.
6. **Commit.** `-m` repeatable (git paragraphs); `Agent: <name>` via `git --trailer` when `DOTWARDEN_AGENT` is set; no `-m` opens `$VISUAL`/`$EDITOR` only when stdin and stdout are terminals (else exit 2); empty message exit 2; nothing staged exit 1. `--amend` without `-m` keeps the message; refused (exit 1) when `git branch -r --contains HEAD` is not empty.
7. **Diff** is generated in-process (old = rendered committed version, new = HOME; staged = HEAD vs index). In a terminal it is piped to the bundled delta; `--no-pager`, `--json` and pipes give plain unified diff. Incoming and Conflict have no diff yet.

### Tests

`tests/changes.rs`, 15 tests with real chezmoi/git: every state; stage+commit leaves no change and repo == index; edit after stage; unstage back to Local with a clean repo; New to stage to tracked and back; assigned-away file; incoming/conflict refusals and `--all` skipping; dry-run and template refusal; commit trailer and multi `-m`; commit refusals; amend and remote refusal; diff plain/json/by path.
`cargo fmt --check`, `clippy -D warnings`, full `cargo test` and the PTY smoke pass. One T2 assertion changed (status heading text for New files).
Hand-broken once: Local classified as Conflict (11 tests failed); stage without `git add` (9 failed). Both reverted.

### Gaps

- Hunk/line staging, discard, apply and conflict resolution are later parts; `dw diff` skips Incoming/Conflict.
- `status` runs `chezmoi dump` and `status` each time (also for the TUI's refresh); not benchmarked on large repos. Staging a deletion and templates are refused.
- The editor path and the delta pager are not covered by tests (need a terminal).
- Linux not run.

## Part B: snapshots, discard, deletions, Changes panel

### Built

- `src/snapshot.rs`: snapshots in `<state>/snapshots/<id>/` (dirs 0700, blobs 0600) with `meta.json` (path, kind, mode, size, sha256, time, reason, actor). Constants `RETENTION_DAYS = 30`, `MIN_KEEP_PER_FILE = 5`; pruning runs after each snapshot and keeps anything younger than 30 days plus the 5 newest per file. `dw snapshot list [PATH]`, `dw snapshot restore <id> [PATH…] [--dry-run]` (snapshots what it replaces first).
- `dw discard PATH… [--yes] [--dry-run]`: new pending action `discard` (preview lists the files, guard = HOME fingerprints). Applying takes a snapshot, then `chezmoi apply --force` on those targets only. Restores deleted files, modes and symlinks. New, Staged-only, Incoming and Conflict files are refused.
- Staging a file deleted in HOME untracks it (staged, same path as `dw untrack`); unstaging it brings the file back as Local `deleted`.
- TUI Changes panel (key 3): tabs Local (incl. New) / Staged / Incoming / Conflicts on `[` `]`. Space, `a`, `c` (box; Enter commits, Ctrl+J or Alt/Shift+Enter newline, Esc cancels), `A` (box prefilled), `d` (y/n), Enter. The main view shows the diff through delta, converted with `ansi-to-tui`. Status panel shows counts. Everything runs on jobs and the action queue; the 2 s tick now also refreshes status.

### Decisions (please check)

1. **Approval rule.** Agents (`DOTWARDEN_AGENT`) are always routed to the pending list (exit 3), even with `--yes`. Humans: `--yes` discards directly; on a terminal without `--yes` there is a y/N prompt; without a terminal and without `--yes` it goes to pending (exit 3). `dw pending apply` is the approval (no `dw approve` yet).
2. **Snapshot scope.** Taken for discard and for snapshot restore. Other HOME overwrites (apply of incoming changes) do not exist yet and must call `snapshot::take`.
3. Staging a deleted file runs the whole untrack (assignments and folder records go too). The chezmoi record clean-up runs after the lock is released (running it under the lock waits out the 10 s timeout).
4. The TUI's Incoming and Conflicts tabs show a note instead of a diff; Space and `a` there say they are read-only. `z` undo is not implemented.
5. Help overlay lines were condensed to three Changes entries so it still fits 24 rows; one T2 TUI assertion changed (Changes placeholder text).

### Tests

- `tests/changes.rs` +6: discard snapshots first and the snapshot restores the edit (0700, sha256); deleted/mode/symlink discard; agent and no-`--yes` routes to pending then approval applies (exit 3); discard refuses New; staging a deleted file untracks and unstage reverts; retention (7 old snapshots keep 5 newest per file, 90-day-old sole copy kept).
- `tests/tui_changes.rs` (5): Space/`a`/tabs and status counts; commit box with multi-line message, empty refusal, amend prefill; `d` n then y with snapshot; diff is styled spans (no escape codes left); Incoming read-only.
- Hand-broken once each: snapshot skipped before discard (2 tests failed); approval gate removed (1 failed). Reverted. fmt, clippy `-D warnings`, full suite and PTY smoke pass.

### Gaps

- No scrolling of long diffs in the TUI; the diff job runs delta with fixed flags (`--no-gitconfig`, 256 colours) and ignores the user's delta config.
- `dw diff` in a terminal still pipes straight to delta (its own pager); the TUI path captures output.
- Commit box has no cursor movement (append/backspace only). Mouse and `v` range select in Changes not done.
- Linux not run.
