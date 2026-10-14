---
name: dot
description: Inspect private dotfiles, review exact Track/Adopt plans, and use the Go dot CLI with machine-scoped approval.
---

# Use dot

Use the installed native `dot --help` before operating; command flags follow the
verb. For verb syntax, JSON schemas and retired operations, read
[`docs/native-distribution.md`](../../docs/native-distribution.md).

## Inspect and approve

- Confirm the user's source, HOME, tracker config and local state. Private
  configuration and machine inventories do not belong in public artifacts.
- Inspect machine-scoped status/diff/scope through the shared engine. Empty
  adoption scope is not whole-machine parity.
  Use `dot status --format json`, `dot diff --format json`,
  `dot scope show --format json`; these are offline and read-only.
- Track imports selected HOME files; Adopt applies selected repository entries.
  Navigation and selection do not grant authorization. Directory selections are
  exact snapshots, not permission to enroll future files.
- Review the exact plan's direction, paths, bytes/modes and effects. Request
  approval for that plan ID; changed inputs require a fresh plan.
  Run `dot intent plan --request FILE`, `dot intent preview --plan FILE`, then
  `dot intent apply --plan FILE --approve EXACT_ID` only with approval.
- Unsafe effects, desktop effects and applicable backup conflicts have separate
  gates. Cancel/Finish is valid. Preserve dirty work, machine identity, approval
  records and recovery data.
- Setup, dependency installation, private pull/push and configuration adoption
  are distinct operations. Never silently apply, commit, push or install.
- For per-machine retargeting, local deselections/customs or profile unions, read
  [`docs/machine-assignments.md`](../../docs/machine-assignments.md). Update
  preserves exact prior enrollment; a future profile addition needs Adopt review.
- Pending recovery blocks apply. Inspect `dot recovery`; exact rollback uses
  `dot recover --approve EXACT_PLAN_ID` and refuses conflicting human edits.

For artifact trust/update/uninstall, use native-distribution's owned-root commands;
public updates never authorize private configuration. For release-readiness claims,
read [`docs/native-acceptance.md`](../../docs/native-acceptance.md), including the
explicit Linux runtime waiver. Unsupported helpers remain fail closed.
