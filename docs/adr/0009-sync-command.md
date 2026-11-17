# One sync command on top of the lazygit change model

`dw sync` runs pull (fast-forward), apply of safe incoming files, stage of local changes, commit with a generated message and push, stopping at the first step that needs a decision (a conflict, a stale state, a secret, a refused remote). The lazygit model (ADR 0003) stays the way to review and shape changes; sync is the habit-sized path for "back up what I changed", because a backup that takes five commands is a backup that does not happen. To keep the goal that every change is visible before it is committed or pushed, sync prints its plan (files staged, applied and skipped, the commit message, the push target) and asks for confirmation in a terminal; `--yes` skips the question for humans, the TUI shows the same plan in a dialog, and an agent's sync commits locally and turns the push into a pending item (ADR 0007).

## Considered options

- Outbound-only `dw backup` (stage, commit, push): rejected because a machine that has not pulled hits a push rejection on its first backup after another machine changed something.
- No new command (TUI `a`, `c`, `P`): rejected because it leaves the CLI, scripts and agents without a one-step backup.

## Consequences

- Sync never applies pending items, never stores a flagged secret in plaintext and never resolves conflicts; those stay explicit.
- Commits made by sync are ordinary commits: `dw undo` reverts the last one while it is on no remote.
