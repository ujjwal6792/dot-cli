# T18 — Import from a bare repository or a path list

## Built
- `dw import --bare DIR | --from-file LIST|- [--dry-run] [--apply] [--json]` (`src/import.rs`,
  `cmds::import_cmd`; clap requires exactly one source). `--bare` validates the repo
  (`rev-parse --git-dir`), reads `rev-parse --short HEAD` and `ls-files -z` with the bundled git,
  `--git-dir=DIR --work-tree=$HOME`, cwd HOME, `GIT_OPTIONAL_LOCKS=0`. Lists: one path per line
  or NUL-separated; blank and `#` lines ignored; relative = HOME-relative, `~/x` and absolute
  inside HOME accepted.
- Sorting out: outside HOME / `..` / HOME itself / symlinked parents → skipped with
  `resolve_home_path`'s reason; missing → "does not exist"; hidden → track's hidden text. A
  listed folder stands for its files.
- Collapsing (`import::group`), then the paths go through `track::plan_each` (new: `plan_paths`
  without the "nothing plannable is an error" rule), so items, previews, secret handling with or
  without a vault, `--apply`, `--dry-run`, agent rules and "Skipped:" are exactly `track`'s
  (`cmds::submit_with` = `submit_skipping` plus extra JSON fields `source`, `label`, `listed`,
  `grouping`). Without a vault, flagged files inside a collapsed folder (which `track` leaves
  out) are also listed under skipped. Nothing added → exit 1 (also `--dry-run`, `--json`).
- Human output: `Import from ~/.cfg at b507421: 9 listed path(s) → 3 tracked folder(s), 2 single
  file(s), 3 skipped`; `--dry-run` adds "Tracked folders:" (with file counts) and "Single files:".
- Items carry `import: {from, head}` (`#[serde(default)]`; old pending.json loads); `pending show`
  prints `source: Import from …`.
- Commit prefix: applying items with a source (`track::apply_review`) records label + target
  paths in `import.json` (state dir); `changes::commit` deletes it. Sync's generated message
  (plan and run) and the TUI `c` prefill use the label while one recorded path is staged.
  Message: `Import from ~/.cfg at <hash>`, blank line, the usual `sync from …` line.
- Skill: import workflow (dry-run, show skips, import, review pending, apply, sync).

## Decisions
- Folder collapsing (§5a "deepest such folder wins, never HOME"): taken literally, every
  subfolder of a complete folder is complete too, so "deepest" would split `nvim` into
  `nvim/lua` + single files. Interpretation: a complete folder holding listed files directly
  is chosen (it covers its subfolders); a complete folder holding only subfolders gives way to
  them. So fully listed `.config/nvim` with `.config` partly listed → `~/.config/nvim`; a fully
  listed `.vim` holding only `colors/` and `autoload/` → those two. Recorded in spec §5a.
- Completeness is checked with `track::walk_folder` (same hide/skip rules as tracking);
  an unreadable folder counts as incomplete. Secrets do not block collapsing.
- The source is shown as `~/…` below HOME (no user name in commit messages); stdin is `stdin`.
- The JSON field is `import`, not `source`: Item flattens Action, and variant actions already
  have a `source` field (caught by the full suite; unit test added).
- One marker at a time: a later import with another label replaces it. `dw undo` of the import
  commit does not bring the prefix back.

## Tests
- `tests/import.rs` (4, real git/chezmoi): bare repo `~/.cfg` over an isolated HOME (nvim fully
  listed, fish partly with `functions/` fully, `.cache` path, gh folder with a `gho_` token, a
  listed file deleted) — grouping, skips with reasons, agent import adds 5 items with source,
  apply + `dw sync --yes` to a local bare remote, commit message prefix, marker cleared, bare repo
  snapshot (bytes + mtimes) identical, token absent from history, outputs, pending.json and the
  repo; `--from-file -` via stdin (outside-HOME skips, `.vim` giving way to subfolders); nothing
  added → 1 and `--bare` on a non-repo → 2; TUI `c` prefill.
- Unit: an import source next to an action `source` round-trips; old items load.
- Hand-broken: index refresh before `ls-files` (bare snapshot assert failed), completeness `all`
  → `any` (grouping assert failed), secret listing removed (skip assert failed); restored.
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, full `cargo test` green.

## Gaps
- With a vault (secrets moved as in `track`) is not tested here (needs an interactive unlock);
  the path is track's own.
- Gitlinks (submodules) listed by `ls-files` are treated as folders; not tested.
- Very large unlisted trees under a listed folder's parents (`.local/share`) are walked once
  per level while collapsing; fine for dotfiles, not optimised.
