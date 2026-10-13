---
name: dot
description: Inspect private dotfiles, review exact Track/Adopt plans, and use the Go dot CLI with machine-scoped approval.
---

# Use dot

The native runtime is under construction (#16–#18). Read installed `dot --help`
first and use only implemented commands. The retired Bash/Python runtime is not
a fallback; report unavailable operations instead of performing them directly.

## Inspect and approve

- Confirm the user's source, HOME, tracker config and local state. Private
  configuration and machine inventories do not belong in public artifacts.
- Inspect machine-scoped status/diff/scope through the shared engine. Empty
  adoption scope is not whole-machine parity.
- Track imports selected HOME files; Adopt applies selected repository entries.
  Navigation and selection do not grant authorization. Directory selections are
  exact snapshots, not permission to enroll future files.
- Review the exact plan's direction, paths, bytes/modes and effects. Request
  approval for that plan ID; changed inputs require a fresh plan.
- Unsafe effects, desktop effects and applicable backup conflicts have separate
  gates. Cancel/Finish is valid. Preserve dirty work, machine identity, approval
  records and recovery data.
- Setup, dependency installation, private pull/push and configuration adoption
  are distinct operations. Never silently apply, commit, push or install.

Native behavior and acceptance ownership are recorded in
[`docs/native-rewrite.md`](../../docs/native-rewrite.md).
