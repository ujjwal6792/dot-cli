# T2 report — Track, untrack, tracked folders, pending list (branch `t2`)

## What was built

**Library**

- `src/pending.rs`: the pending list. `pending.json` in the state dir (`{version, next_id, items}`), written atomically; every mutation takes the store lock (waits up to 10 s). Items have `id` (sequential, never reused), `kind` (`track`, `untrack`, `hide_add`, `hide_remove`), `requester` (`DOTWARDEN_AGENT` or `human`; the TUI is always `human`), `created` (UTC RFC 3339), `summary`, an exact `preview` and a `guard`. Identical requests collapse to the existing item (`already_pending`). A corrupt file is reported, never overwritten.
- `src/source.rs`: decodes the repository's source tree into HOME target paths (tracked files, symlinks) and reads/writes the tracked folders (`dotwarden.tracked_folders` in `.chezmoidata/dotwarden.toml`). `state_of()` gives tracked / tracked-folder / contains-tracked / new / untracked.
- `src/track.rs`: path resolution, folder walking, planning, freshness, the transactional apply engine, toggling (TUI Space) and `ls`.
- `src/cmds.rs`: CLI handlers. `src/hide.rs`: rule validation/editing, `for_store`, `why_hidden`.
- `status` now reports the real pending count, tracked file/folder counts and `new_files` (untracked, unhidden files inside tracked folders).

**CLI** (all with `--json`, help text, typed exit codes; mutating ones with `--dry-run`)

`dw track PATH… [--apply]`, `dw untrack PATH… [--apply]`, `dw pending [list|show ID|drop IDS…|--all|apply IDS…|--all]`, `dw hide [list|add PATTERN…|remove PATTERN…] [--apply]`, `dw ls [PATH] [--all]`.

**TUI**

- Files: Space toggles track/untrack (adds or drops a pending item; no dialog); `v` range select then Space (each entry flips independently; Esc cancels). Markers: `✓` tracked, `▣` tracked folder, `◇` folder holding tracked files, `?` New, blank untracked, `@` symlink suffix, second column `+`/`-` for a pending track/untrack. The main view shows `[state] pending: …` above the preview.
- Pending pane (bottom right) lists `[x] #id requester: summary`. Focus it with `0` or Tab/Shift-Tab (it follows Machines in the Tab cycle; panels 1–5 keep their numbers). Selecting an item shows its full preview in the main view. `Space` checks/unchecks (all items start checked), `d` drops, `Enter` opens one review dialog listing every checked item with its files and data changes; Enter/`y` applies, Esc cancels. A result dialog shows per-item outcomes (applied / stale / failed).
- All I/O is on jobs. Mutations (toggle, drop, apply) go through an ordered queue of detached jobs so they never supersede each other; the toggle is decided under the lock, so quick repeats and concurrent CLI use stay consistent. Footer shows `working…` while actions run.

## Decisions and interpretations (please check)

1. **Exact preview** comes from chezmoi itself: `chezmoi add --dry-run --verbose` yields the real source paths (`private_`, `executable_`, `empty_`, `literal_`…). The preview stores paths/actions only, never file content (secrets, T8). After the real add, dotwarden verifies that exactly the previewed paths (plus the data file) changed; any difference rolls the item back.
2. **Staging choice** (spec left open): applying runs `git add -A -- <exact changed paths>` only; nothing is committed. The repository must be clean apart from staged changes (no unstaged edits or untracked files) or apply refuses with exit 1; dotwarden never touches stray repo edits.
3. **All-or-nothing per item**: the index tree is recorded, and on failure `git add -A` + `git read-tree --reset -u <tree>` restores index and working tree. Other items in the batch still apply; results are listed per item.
4. **Freshness / exit 7.** Each item stores a guard: HOME fingerprints (type, size, mtime ns, mode, link target) of the files it will add, and repository facts (is each target tracked and by which source file, hide-rule match per file, folder recorded, assignment present, rule present). Apply recomputes it; any difference → `stale`, item kept, reason shown. Any stale item makes the command exit 7 (stale wins over failed=1), while fresh items still apply. `pending list/show` display the stale reason. Untrack does not check HOME (HOME is left alone). Guards are per item, so items in one batch do not stale each other (except real overlaps).
5. **Tracked-ness is decoded from the source tree**, not `chezmoi managed`, because `managed` omits files assigned away from this machine (and tracking them again would overwrite the shared version). Cross-checked against chezmoi in `the_source_decoder_agrees_with_chezmoi_managed`. chezmoi's attribute rules (each prefix once, `.literal` suffix) are replicated for what `add` produces.
6. **Tracked folder rules.** Tracking a folder records it and tracks its current unhidden files, skipping nested `.git` folders, sockets/special files, unrepresentable names and files already tracked. Re-tracking a recorded folder picks up its New files (it errors "nothing to track" if none). A folder under an already recorded folder is not recorded again. Symlinks, including symlinks to folders, are tracked as symlinks and never followed. Tracking below a symlinked folder is refused.
7. **Untrack** removes the source files itself (not `chezmoi forget`, which cannot see assigned-away files), prunes empty source folders, removes the folder record(s) at/below the path, and removes the assignments of the removed files. Untracking a file inside a tracked folder keeps the folder; the file shows as New again (the preview says so). chezmoi's entry state for the path is left as is. Variants do not exist yet (T6 must clean them on untrack).
8. **Exit codes for refusals**: outside HOME, HOME itself, dotwarden's own dirs, unrepresentable names, symlinked parents, bad hide pattern → 2 (usage). Hidden path, nonexistent, already tracked, not tracked, unknown pending id, duplicate/unknown hide rule → 1. Track/untrack never return 3 (they are permitted agent actions). With several paths, nothing is added if any is refused, and all reasons are reported.
9. `--apply` applies freshly planned items directly without storing them in `pending.json`, so a failure leaves no junk item. `pending apply` requires ids or `--all`. `dw ls` without a path lists the current folder when inside HOME, else HOME.
10. `hide add` of a pattern matching already tracked files warns that they stay tracked. Un-hiding uses a negated rule (`!/path/`), which T1's `HideRules` supports for literal paths.
11. The data-file key is `tracked_folders` (array of target paths) under `[dotwarden]`; spec §3 now records the verified mechanics (item 9).

## Tests

Host macOS arm64, real bundled chezmoi/git, isolated HOME/XDG per test.

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test`: 62 unit, cli 17, git_isolation 6, git_wrapper 4, isolation 4, registry 3, spike 20, **track 35**, tui 12, **tui_track 15**; all pass. `python3 -I scripts/pty-smoke.py target/debug/dotwarden` passes (now also presses Space and checks the pending list afterwards).
- `tests/track.rs` covers: pending → apply → staged-not-committed; preview equals what is staged for private/executable/empty/literal/odd names; dry-run changes nothing (repo snapshot, no `pending.json`, chezmoi state); `--apply`; agents as requester; folders (record, hidden children, nested `.git`, New in `status`/`ls`, re-track); symlinks (file, to-folder, inside folder); every refusal; multi-path atomic refusal; duplicates; relative paths/cwd; untrack file/folder/in-folder/not tracked/assigned-away; decoder vs `chezmoi managed`; pending list/show/drop/ids/apply selection/status count/corrupt file; stale by HOME change, repo change and hide change (exit 7, item kept, fresh items still apply); failed item with others applying (repo left exactly as before); rollback after files *and* data were written (content digest equal); unstaged repo edits block apply; 8 concurrent CLI processes; hide add/remove via pending, negation, warnings; user's own chezmoi dirs untouched; JSON error shape.
- `tests/tui_track.rs` covers: Space add/drop with markers and footer, untrack on tracked, folder track and all markers, hidden refusal, `v` ranges (mixed track/untrack, Esc, upward), Pending pane (requesters, preview in main view, `d`, Tab/`0`), review dialog (uncheck, cancel, confirm, result), stale in partial results, empty selection, narrow 45×14, CLI changes after `r`, help text, slow chezmoi (Space returns <150 ms, keys keep flowing, queued actions run in order), TUI + concurrent CLI consistency.
- Mutation checks (each made the named tests fail, then reverted): no staging; stale check off; rollback no-op; no mutation lock (concurrency test); folder not recorded; hidden check removed; toggle never drops; actions run synchronously on the loop (non-blocking test).

## Known gaps / not verified

- Linux not run (AGENTS.md). Code uses `std::os::unix` only; nothing Linux-specific was added.
- TUI `i` (hide from Files), mouse/click focus of Pending, and reloading hide rules in a running TUI after a CLI `dw hide` change (restart or none until next start) are not done; the hide rules are loaded once at TUI start.
- Large folders: preview/apply chunk chezmoi and git calls (400 paths), and the dry-run runs under the lock; tracking tens of thousands of files was not benchmarked.
- A hide rule that starts matching a *tracked* file does not untrack it (warned only); `chezmoi` entry state for untracked paths is not cleaned.
- No secret detection on track yet (T8): previews contain no file content, but nothing flags secrets in tracked files.
- Per-item errors that happen before any item runs (unparseable data file, git failure while checking cleanliness) abort the whole apply with exit 1 and no per-item list.
- `pending.json` has no schema migration beyond refusing an unknown `version`.
- The PTY smoke script now waits for stable text because ratatui redraws as a diff (changed from `.cache` / `Help (Esc to close)`).
