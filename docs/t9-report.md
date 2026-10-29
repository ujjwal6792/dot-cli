# T9 report (branch `t9`)

## Built

- `src/conflicts.rs`: line-based 3-way merge on `similar` (base/local/incoming → Same/Merged/Conflict chunks, picks local/incoming/both), base lookup, resolution, editor session, `plan_resolve`.
- CLI `dw resolve PATH --take local|incoming | --merge [--dry-run] [--json]`. Result: snapshot (reason `resolve`) → HOME → last-written set to the committed version → normal `stage` (so references, variants and secrets use the existing reverse mapping) → no longer a conflict. `changes::stage_with` lets a caller that holds the lock stage.
- Pending action `resolve {path, merge}` (approval-required). Agents: `--take incoming` and `--merge` → pending item, exit 3; `--take local` runs (it only stages HOME).
- TUI: Enter on the Conflicts tab opens the hunk screen (hunk list + base/local/incoming panes, masked); `j/k`, `l/i/b`, `e`, `w`/Enter then y/n, Esc. Load and write run as jobs; only the editor handoff blocks (`run_loop` restores the terminal, pauses the input thread, re-enters the alternate screen).
- Editor: `$VISUAL`/`$EDITOR`; nvim gets `-d merge local incoming`. Files live in a 0700 `state/merge-PID/` (0600 files), removed on drop. The result is refused while `<<<<<<< local` / `>>>>>>> incoming` lines remain.

## Decisions (please check)

1. **Base** is found by hash: the last-written SHA is matched against the committed versions (last 60 commits) of the source file and of the variant files; templates and reference-bearing variants are rendered with current values first. Without a base (e.g. a value changed since) every difference is a hunk ("no base").
2. **Merge on rendered text**, not stored form: HOME holds rendered content, and staging maps it back via the existing reverse mapping. Display masks known secret values (`«secret:NAME»`); a locked vault hides the file from classification (exit 4), so a secret conflict needs an unlock. Choosing a side of a secret line stages that side's value through the vault as `stage` does.
3. **Non-interactive `--merge` with overlaps** exits 5 listing hunk numbers and base lines only (no content). Markers exist only in the private merge file. Adjacent line edits count as overlapping (as in git).
4. Agent `--merge` also needs approval (it rewrites HOME); an agent cannot queue one that has overlaps.
5. Human take-incoming/merge need no prompt (snapshot protects); only agents are gated.
6. Symlinks, missing or non-UTF-8 files: `--take` only.

## Tests

`tests/conflicts.rs` (6, two machines, local bare remote): take local (+ refusals, round trip to alpha); take incoming with snapshot restore; agent pending/exit 3/human approval, agent take local; clean merge of distant edits (+ dry run); overlap exit 5 for human and agent, no markers anywhere in HOME; variant (platform:darwin) conflict, overlap refused, merge lands in the variant file not the default.
`tests/tui_conflicts.rs` (4): l/i picking with auto-merged hunks and n/y prompt; Esc; secret conflict (CLI exit 5 text/JSON and screen free of the value, reference kept in repo); editor session (perms, removal, markers refused, nvim diff args). Unit tests in `conflicts.rs` (merge engine).
Hand-broken: clean merge returning marker text (2 failed); masking disabled (1 failed). Reverted.

## Gaps

- Editor launch itself (`hand_off`) is not run in tests (needs a real terminal); the session/command parts are.
- No whole-file `L`/`I` keys or `b` hunk preview in the TUI; no separate TUI route for take-incoming (use hunks or the CLI).
- Linux not run.
