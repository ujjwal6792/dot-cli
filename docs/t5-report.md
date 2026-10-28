# T5 report (branch `t5`)

## Built

- `src/machines.rs`: plan/guard/exec for `assign`, machine add/rename/retire, `list`, `preview_as`. `src/assign.rs`: `Assignment`, `applies` (file + ancestor folders), `Known` target validation, tri-state `editor_rows`, `effects`.
- Pending actions `assign`, `machine_add`, `machine_rename`, `machine_retire` (only retire `needs_approval`). `Preview.effects` lists files that start/stop applying per machine.
- CLI: `dw assign PATH… [--include|--exclude|--unset T]… [--clear] [--apply|--dry-run]`; `dw machine list|add|rename|retire|preview [--diff]`.
- TUI: Machines panel (5) lists the registry, Enter shows the preview in Main; `m` in Files (also `v` ranges) and Changes opens the editor (Space cycles, `i`/`x`/`-` set, Enter adds one pending item); Files badge `[darwin,!srv]`.
- Spec §4 rewritten to the behaviour below.

## Decisions (please check)

1. **`dw assign` semantics.** `--include/--exclude` add to the existing lists (a target moves sides); `--unset` removes one target; `--clear` empties first, so `--clear --include X` replaces. Item key includes the final lists, so two different requests for one path are two items; the second goes stale (exit 7) once the first applies.
2. **Ancestors.** A file applies only if its own and every ancestor folder's assignment allows it (matches chezmoi ignoring a folder). Verified through real chezmoi in a test.
3. **Validation.** Unknown platform/machine and retired machines: usage error (2). Unseen distribution/family: warning in the preview. Untracked paths: refused.
4. **Add and rename are pending items** (staged, not committed, like every other item) — not committed at once as `init`/`clone` registration is.
5. **Retire** removes the machine only from assignments; variants/overrides naming it stay (listed as notes). Removing a file's only include frees it for all machines; the preview shows it as "starts applying".
6. **Rename** rewrites `machine:<old>` strings and keys anywhere in the data file (so future value overrides are covered). Guard = registry state + data-file hash.
7. **Preview as** uses `chezmoi managed --path-style=relative` / `diff --use-builtin-diff` with a temp config/state/cache under `$TMPDIR` (removed afterwards). Tests snapshot HOME, state and repo before/after.

## Tests

`tests/machines.rs` (10, real chezmoi/git; two-machine round trips through a bare remote): assignment keeps file off beta after pull+apply; exclude beats include; folder covers descendants + stop/start effects; target validation and warnings; odd paths (`[1]`, `#`, `!`, trailing space, `*`); rename (registry, targets, variant override, this machine's config, other machine still matches); retire (agent gets exit 3, this machine refused, targets removed, retired not assignable); preview-as writes nothing and diffs; agent may assign and stale item; add/rename checks. `tests/tui_machines.rs` (3): tri-state editor on mixed selection (mixed target untouched, no-change Enter), editor from Changes + untracked refusal, badges + Machines panel preview.
Hand-broken: retire's approval flag removed (1 failed); ancestor check removed (1 failed). Reverted. The tri-state test also caught a dialog-dropped-on-empty-Enter bug during development.

## Gaps

- A computer whose registry name was renamed/retired elsewhere must be re-registered by hand (no command yet); unstaging a staged rename leaves `machine.toml` on the new name.
- Editor dialog does not scroll: a registry with many distributions/machines overflows a 24-row terminal.
- Value overrides do not exist yet; rename covers them only generically.
- Linux not run.
