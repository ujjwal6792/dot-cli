# T11 part A — pull, history and Commits

## Built
- Default pull fast-forwards or rebases unpushed commits; a conflicting rebase aborts and names the stored files. HEAD, index, stored content and HOME rollback are tested.
- `pull --merge` retains conflicts; `resolve PATH --take local|incoming` or `--merge` resolves the stored form. `pull --continue` commits; `--abort` restores. HOME is never written.
- Conflicted stored files (including registry data) appear in Conflicts. T9's masked hunk picker/editor is reused; repository resolution preserves executable mode.
- `log [PATH] [-n N]`, `show COMMIT [PATH]`, `revert COMMIT`. History includes author and Agent trailer; known/detected secrets are masked in metadata and diffs. Show uses delta on a terminal.
- HOME-path history includes earlier stored spellings and deleted files. Revert is a normal local commit, permitted for agents with their trailer; conflicting reverts abort.
- Git panel (4) Commits: first-parent list, ● commits / ◆ merges, cursor diff, 50-at-a-time expansion. `r` prompts y/n to revert; elsewhere `r` still refreshes.
- `p` uses default pull. Git `M` merge-pulls, `C` continues, `X` aborts; a stopped merge focuses Changes → Conflicts. Help and CLI text updated.

## Decisions
- Spec §6 updated per #33/T11A: diverged default is now rebase rather than the original explicit-choice-only rule.
- Diverged pulls require no staged changes. Commits reachable from any known remote-tracking ref are not rewritten; local merge commits also require `--merge`.
- Repository conflict resolution is local staging (agents allowed); T9 HOME-conflict approval rules remain distinct.
- Log and TUI use first-parent history; reverting a merge uses parent 1. Ordinary editing/staging is blocked while a repository merge is in progress.

## Verification
- macOS; isolated HOME/XDG, file session backend. Shared integration-test environments now explicitly select the file backend.
- 11 new end-to-end tests plus the former divergence-refusal test updated: rebase/push, exact content rollback, merge/resolve/continue/push, abort, published-commit protection, registry conflicts, clean merges, history/masking/revert, TUI paging/diff/revert and stored hunk resolution.
- Hand-breaks: omitting rebase abort failed the rollback test; disabling history-diff masking failed the secret canary. Both restored.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.
- All 332 tests pass, plus doc tests (0). Initial full run found missing CLI argument help, fixed. The subsequent suite hit the shell timeout during vault tests; only that unfinished target and doc tests were resumed.

## Limits
- Binary, symlink and delete/modify conflicts use whole-side selection, not text hunks. Remote-tracking refs are the evidence of publication; deleted/unknown remote history cannot be inferred.
- Historical masking uses known secret values and existing secret detectors; arbitrary legacy plaintext unknown to the vault is not guaranteed detectable.
- Real terminal editor handoff and interactive delta display were not manually exercised; Linux was not run. No branch/stash/restore-at work (part B), no push.
