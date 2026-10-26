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
