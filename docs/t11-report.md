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
- At part A's commit: real terminal editor/delta display and Linux were untested; part B was pending. No push.

## Part B — built
- CLI `branch list/create/switch/delete`: source/index-only switching, overlap refusal, default-branch warnings, guarded pending approval for agent deletion; current deletion refused and unmerged deletion requires explicit force.
- CLI `stash list/push/pop/drop`: exact stored index commit under local `refs/dotwarden/stashes/` plus separate tracked HOME payload. Partial staging, references, variants, and staged tracking additions/removals round-trip. Untracked HOME is excluded.
- Push/pop list HOME/index paths before confirmation and snapshot before overwrite. With a vault, payloads and new safety snapshots are encrypted before disk writes; locked operations refuse. Without one, detected secret-like content refuses capture. Drop uses pending approval for agents.
- `restore --at REV PATH…|--all` renders an isolated historical source copy using historical data/variants and this machine's identity; both historical/current assignments must receive the file. Candidates contain hashes/fingerprints, never rendered secrets.
- Candidates appear separately in Incoming/Conflicts and `diff`; HOME/index/HEAD stay unchanged until apply. Apply snapshots HOME, leaves index/HEAD intact, and makes restored content a Local edit. Unrelated forward Incoming survives. Stale/colliding candidates refuse; `restore --clear` removes them.
- Git `[ ]` tabs: Commits, Branches, Stash. Branches `n` create, Enter switch, `d/D` delete/force; Stash `s` push, Enter pop, `d` drop. Commits `R` previews a historical restore. Mutations use background jobs, path previews, y/n and preview guards; no new event-loop I/O.
- JSON, dry-run and help added; spec §6/§11 documents scope. Temporary historical copies are clones, not Git worktrees; transient file-backend unlock sessions are cleaned up.

## Part B — verification
- 14 new behavioral tests: 11 CLI/real-Git tests and 3 TUI tests. Branch index/HOME preservation, approvals, partial/new/removed tracking stash state, conflict retention, references/variants, historical assignments, local/stale collisions, encrypted canaries and UI confirmation are covered.
- Encryption hand-break: bypassing protected snapshot encryption failed the secret canary; restored. Together with part A this is three targeted hand-breaks.
- Final full run completed (no timeout): 345 passed, 1 failed, doc tests 0. Failure was read-only status updating Git's index cache via `write-tree`; replaced the guard with a read-only cached-diff fingerprint.
- Post-fix targeted checks: read-only regression 1/1, git_depth 11/11, new Git TUI actions 3/3 pass. All 346 distinct tests have passing coverage; the already-passing full targets were not repeated.
- macOS only, isolated HOME/XDG and file sessions throughout. Post-fix `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` pass.

## Part B — deliberate limits / stop boundary
- Stash pop requires its original HEAD and a clean source index; it retains the stash rather than attempting a cross-base merge. Custom templates, non-UTF-8 files, symlinks and staged vault identity changes refuse explicitly.
- Historical restore is limited to regular UTF-8 files received both then and now, with Dotwarden-owned templates. Historical rendering requires unlock if its render scope needs secrets; removed/unassigned targets are not resurrected.
- No crash-recovery automation for an interrupted multi-file stash; recovery payload/index refs and safety snapshots are retained. Interactive terminal/editor testing and Linux remain unrun.
- Work stops after T11B. No T10 or other ticket, merge, push, or GitHub changes.

## Follow-up — file history and Remotes tab
- `H` in Files or Changes shows that file's history (old spellings included) on Commits, with per-commit diffs limited to it; `Esc` returns to all commits. Spec §10 key list gains `H`.
- Git tabs are now Commits / Branches / Remotes / Stash (spec §10). Remotes `n` adds (NAME URL), `d` removes; both preview then y/n. Known hosts are probed in the preview and public ones refused; for unverifiable hosts the human's y is the one-time private confirmation. Agents get pending items.
- 2 new TUI tests (file history, remote add/confirm/remove); stash test updated for the 4th tab. Hand-break: ignoring the path in the TUI log failed the history test; restored.
