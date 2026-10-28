# T7 report (branch `t7`)

## Built

- `src/values.rs`: variable model (one data file per variable), `dotwarden/value` helper (`assets/home/.chezmoitemplates/dotwarden/value`), reference parse/escape, `wire`, reverse mapping (`reverse`, character diff), staging plan, detection, listing.
- Pending actions `value_set`, `value_unset`, `value_wire` (no approval; agents may add and apply). Preview shows files, data, per-machine effects and files using the variable. Machine rename rewrites `machine:<old>` overrides.
- CLI: `dw values list|show|set-variable|unset-variable|wire` (`--apply`, `--dry-run`, `--json`).
- Stage: files with references (plain `.tmpl` and variant files) are rebuilt from HOME; changed values update the variable in the same staged change. `dw status`/`dw diff`/stage output say "variable NAME changed".
- Detection after `track` and `stage` proposes pending wire items. TUI `V` screen (table, edit default, `o` override, `n` new, Esc).
- Spec §3 (item 10, variant template, helper), §7 updated.

## Decisions (please check)

1. **Reference syntax** is `{{ includeTemplate "dotwarden/value" (dict "data" . "name" "NAME") }}`. The `template` action is undefined inside files run by `includeTemplate` (verified), and variant files must be rendered that way.
2. **Variants**: the T6 stored template now renders a variant file as a template only when it contains the reference marker, else raw (changed `variants::template`). Raw variants containing the marker text are refused on stage.
3. **Data**: `.chezmoidata/dotwarden-values/<NAME>.toml` per variable (merge-conflict free); names unique ignoring case; an override needs a default first.
4. **Reverse mapping** by character diff; an edited reference span takes the whole edited region (adjacent inserts included). Insertions next to an unedited value stay literal (`12` → `123` renders the same, no variable change).
5. Staging updates the most specific target this machine matched; other files using that variable show Incoming afterwards.
6. Detection ignores values shorter than 3 characters.

## Tests

`tests/values.rs` (10, real chezmoi/git, two machines over a bare remote): override per machine and specificity; wire renders the same and leaves HOME alone; unchanged stage is a no-op; value edit reported then staged into the matched override; literal braces through wire/stage/apply; references inside a variant; detection on track and stage (not auto-applied); agent via pending; refusals/rename/override removal; conflicting text at two positions. `tests/tui_values.rs` (1). Unit tests in `values.rs` (escaping, reverse).
Hand-broken: escaping disabled (2 failed); variable update skipped on stage (2 failed). Reverted.

## Gaps

- The Values screen reads files synchronously on open/after an action (small local reads).
- No TUI flow for wire or unset (CLI only); no hunk-level staging of files with references.
- A mode-only change of a template file is refused with the variant message.
- Linux not run.
