# T6 report (branch `t6`)

## Built

- `src/variants.rs`: data model, Rust mirror of the specificity selection (`pick`), plan/guard/exec of `variant add|remove`, summaries for status/TUI/preview-as.
- Pending actions `variant_add`, `variant_remove` (no approval needed; agents may add and apply). `Untrack` gained `variants` (removes data and files).
- CLI: `dw variant add PATH --target T`, `list [PATH]`, `remove PATH --target T` (`--apply`, `--dry-run`, `--json`).
- Selection through the shared helper; `status`/`doctor`/`variant list`/`machine preview` show the variant received and clashes. Stage, classify, diff, unstage, discard follow the variant this machine receives.
- TUI: `◆N` badge (Files, Changes, `!` on clash), versions list in the main view, `t` opens the target picker (reuses the assignment editor rows), "blocked N" in Status. Help line `m / t`.
- Spec §3 item 6, §4 (Variants, clash), §5 (stage), §10 (`t`) updated.

## Decisions (please check)

1. **Default is a file.** `home/.dotwarden/variants/<path>/default` holds the old stored content; the template includes it, so no escaping of `{{` and staging into the default is a plain file write. The spec's inline-default example was replaced.
2. **Variant content comes from HOME**, the default from the stored file (so a Local edit becomes the variant, not the default). When this machine receives the new variant, chezmoi's last-written record is set so the file is not Conflict later.
3. **Clash scoping.** chezmoi `dump`/`status`/`diff` fail wholly on one ambiguity even with `--keep-going`, so dotwarden detects clashes itself and passes the other managed files explicitly (only when a clash exists). A clashed file is unclassified and refused for stage (naming it explains why).
4. **Staging** a variant file writes to the received variant file; mode-only changes of variant files are not staged. Staged variant sources map back to the HOME path (data-file entry first, then the directory name).
5. **Badge** counts versions including the default (`◆2` = default + one variant).
6. Refused: folders, symlinks, templates that are not variant files, duplicate target, retired/unknown machine targets. `remove` accepts retired machines' targets.
7. `t` chosen for the TUI key (unused in spec §10).

## Tests

`tests/variants.rs` (10, real chezmoi/git, two computers over a bare remote) + 1 in `tests/tui_machines.rs`: different content on two machines and machine beats family/platform (raw `{{` content kept); clash blocks only its file (status, doctor, stage refusal; stage/commit/apply/discard of others work); staging and discard use beta's variant, others untouched, diff; default receiver stages into default; exact add/remove previews and conversion back to a plain file, refusals; agent via pending; rename keeps selection and file names are not reused; retire note and later remove; untrack removes everything; preview-as marks the variant and diffs; TUI badge/main view/picker.
Hand-broken: stage always writes the default (1 failed); clash scope disabled (1 failed). Reverted.

## Gaps

- Variants of symlinks, encrypted files or hand-made inline templates are unsupported (refused with a reason).
- No TUI flow for removing a variant (CLI only); no hunk-level variant staging.
- `retire` note filter `starts_with("assignments / ")` (T5) never matches because paths start with `dotwarden / `; not touched.
- Linux not run.

## Review fixes

1. **Nested apply on a fresh machine.** `apply_incoming` and `discard_files` pass `--parent-dirs` (verified in the pinned chezmoi; snapshot restore already creates parents). Test `applying_on_a_fresh_machine_creates_missing_parent_directories` (3 levels deep, two machines, plus discard after removing the folders). Hand-broken: flag removed from apply (test failed).
2. **Retire note filter** now `contains("assignments / ")` (paths start with `dotwarden / `). Test `retiring_notes_assignments_and_…`; old filter makes it fail.
3. **Mode-only change of a variant file** is reported, not staged: `dw stage` refuses it ("only its mode differs; mode changes of variant files are not supported"; skipped and listed with `--all`). Chosen because the mode lives in the template's name, shared by all variants.
