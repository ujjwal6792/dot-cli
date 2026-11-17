---
name: dotwarden
description: Manage dotfiles, variables and secrets across machines with the dotwarden CLI (`dw`). Use when asked to track, assign, commit or sync dotfiles, or when `dw` reports it needs approval.
---

# Using dotwarden (`dw`)

Run every command with `--json` and read the exit code. Set `DOTWARDEN_AGENT=<your name>`
so commits and the activity log name you. Preview any change with `--dry-run` first.

## You may
- Inspect: `dw status`, `ls`, `diff`, `log`, `show`, `pending`, `machine list`, `values list`.
  Secret values are never shown; do not try to read them.
- Track and untrack files, stage and unstage, assign files to machines, add variants.
  With several paths, `dw track` adds the valid ones and lists refused ones under
  `skipped` (path and reason); it exits 1 only when nothing was added. Read `skipped`.
- Commit (`dw commit -m "..."`); your commits carry an `Agent:` trailer.
- Back up in one step with `dw sync --json [-m "..."]`: it pulls (fast-forward only),
  applies incoming files without local edits, stages every local and new file (files
  with secrets and other refusals are listed under `skipped`), commits, and turns the
  push into a pending item: exit 3 with the commit made locally. Exit 5 (conflict) or 4
  (vault locked) means it stopped before staging; exit 1 on diverged history means
  `dw pull` first. Preview with `dw sync --dry-run --json`. `dw status --json` reports
  `backup` (last push, unbacked changes, `warning`).
- Add secret placeholders and wire references (`dw values add-secret NAME --placeholder`,
  `dw values wire`). A human sets the value.
- Paths are HOME paths (`~/.zshrc`); never touch the repository directly.

## A human must approve
Push (a human's later successful push also clears your pending push item), discard, apply over local changes, history rewrite, branch delete, machine retire,
remote add/remove and privacy confirmations. When you ask, dw adds a pending item marked
"requested by <you>" and exits 3. Do not retry or work around it. Tell the user to run
`dw approve <id>` in a terminal or approve in the TUI, then continue.

`dw clone` run by an agent leaves the "Register machine" commit local and prints
`run dw push to share this machine with the others`; ask the human to push.

## Exit codes
0 ok, 1 error, 2 usage, 3 needs approval (added to pending), 4 vault locked (ask the human
to run `dw vault unlock`), 5 conflict (`dw resolve`), 6 public remote refused, 7 stale
(state changed; re-read `dw status` and retry).

## Never
- Put secret values on a command line or in a file you write; use `dw values add-secret`.
- Remove or edit `~/.local/state/dotwarden` or the repository by hand.
- Add a remote that is public; dw refuses and so should you.
