# T10 report (branch `t10`)

## Built

- `src/hunks.rs`: one model for CLI and TUI. `open(rel, side)` returns the hunks of a file (3 lines of context) with every changed line numbered 1..N over the whole file; `apply(rel, side, selection, expect, dry_run)` writes the result. Stage = committed text plus the selected changes; unstage = staged text minus the selected changes. HOME is never written.
- CLI: `dw diff PATH --hunks [--staged]` (numbered hunks, `--json` gives the `View`), `dw stage PATH --hunk N… --line N…`, `dw unstage PATH --hunk N… --line N…`. `--dry-run` and `--json` as for the other mutations. Bad hunk/line numbers or several paths: exit 2; read-only file: exit 1; vault locked: exit 4.
- TUI: `Enter` on a Local/New or Staged change opens a full-screen staging view (Enter on Incoming/Conflicts is unchanged; on `repo:` entries it says they are staged whole). `j/k` lines, `[ ]` hunks, `Space` stages/unstages the line (or the hunk from its header), `a` the whole hunk, `Esc` back. The view reloads after each action and closes when nothing is left.

## Decisions (please check)

1. **Editable = plain text, tracked, non-template, non-variant, source equal to the rendered target, no detected secret.** Everything else opens read-only and is staged whole with Space: files with references (shown through the existing `masked_diff`, markers `«secret:NAME»`), templates, variants, new files, links, deleted files, files whose HOME text holds a detected secret (those lines are replaced by `«hidden: possible secret»`; known vault values are redacted everywhere). Hunk staging of a reference file would need the reverse mapping per hunk; not attempted.
2. **Last-written record.** After a partial stage/unstage the source (and so the target) changed, so `restore_last_written` moves chezmoi's record to the new target; otherwise the file would classify as Conflict. Done under the mutation lock with rollback of index tree and chezmoi state, like `stage`.
3. **Numbers are per view.** After a partial stage the diff changes, so line numbers restart; the TUI passes the fingerprint of the view it showed and a mismatch exits 7 (stale). The CLI has no `expect`.
4. Mode-only changes are not part of hunk staging.

## Tests

`tests/hunks.rs` (7, real chezmoi/git): one hunk staged leaves the other Local, HOME untouched, then the rest and a commit leaves nothing; single line of a replacement; unstage one hunk and then lines (index == working tree, record consistent); bad selections and dry-run change nothing; new file read-only; detected secret never in human/JSON/stderr and not stageable; vault reference shows the marker, not the value. `tests/tui_changes.rs` (+2): Enter, line then hunk staging with HOME intact; secret file read-only and masked on screen.
Hand-broken: selection ignored (stages everything): 2 tests failed. Reverted. A second break (secret read-only check) was not confirmed (see gaps). fmt, clippy `-D warnings` and the full suite pass.

## Gaps

- No hunk staging for reference files, templates, variants, new files; no `v` range select; no help-overlay line (it is full at 24 rows).
- The read-only-secret check was only covered by the output assertions; my hand-break edit of it did not apply cleanly and was not repeated.
- Unstage view assumes the repository equals its index (ADR 0003 invariant).
- Linux not run.
