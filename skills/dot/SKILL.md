---
name: dot
description: Inspect and manage dotfiles with dot's chezmoi integration, review exact file actions, and publish through Git when requested.
---

Use `dot --help` for the installed interface. `dot status` separates HOME drift,
managed source, and Git changes; `dot diff` shows source → HOME, while
`dot diff --capture` reverses that direction. Native chezmoi evaluation may run
repository templates and hooks; inspection is not a sandbox for untrusted sources.

Use `dot plan ACTION PATH...` to obtain an exact review and ID, then
`dot execute ID --approve ID` within the user's authorized scope. Files, rules or
Git changes invalidate old plans. Empty selections never mean all files.

- Capture imports HOME into source. Existing templates require source editing or
  an explicit mapped-setting change instead of flattening rendered content.
- Apply/restore writes source → HOME; restore explicitly replaces local edits.
- Stop tracking changes shared source and retains HOME. Enable/disable only changes
  this machine's automatic-update scope.
- Stage, commit and push are separate. Commit uses the actual Git index, including
  externally staged work. Publish only when the user has authorized publication.
- `dot activity` and `dot recovery` expose results/backups. A partial failure is
  not an atomic rollback; inspect updated/failed/not-attempted results before retry.

Set `DOT_ACTOR` and `DOT_INTENT` to concise non-secret labels when operating as an
agent. Automatic sync, trusted repository execution and report publishing are
separate local opt-ins. Source migration and switching preserve HOME until Apply.
