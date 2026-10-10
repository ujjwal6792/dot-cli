---
name: dot
description: Interact with the public dot CLI: inspect private configuration state, browse Track/Adopt selections, review exact plans, and request explicit application approval without bypassing safety boundaries.
---

# Public dot interaction

Use the installed `dot` launcher for public `ujjwal6792/dots-cli`; private
`dotfiles` remains the user's source of truth. This skill carries no private
configuration, manifest, history, credentials or machine inventory.

## Resolve and inspect

1. Read installed `dot --help`/usage to identify supported commands; pending
   features are not permission to implement an alternative mutation path.
2. Confirm `DOT_ENGINE` resolves public tooling and `DOT_SOURCE` resolves private
   configuration. Keep the private dedicated `home/` boundary and actual
   `DOT_CONFIG`. Use `dot contract check --engine ABS_PATH --source ABS_PATH`
   for read-only compatibility checking. Missing/incompatible paths stop.
3. Inspect status, scope and read-only plans. Report selected-entry count alongside
   drift; an empty scope and zero drift do not establish whole-machine parity.

## Plan, preview, approve

- Track imports explicitly selected eligible HOME configuration; Adopt applies
  explicitly selected repository configuration. They are distinct workflows.
  Browser navigation/selection is not approval, and directory selections are
  exact eligible-file snapshots, not permission for future files.
- For automation use the shared `dot intent plan REQUEST --out PLAN`,
  `dot intent preview PLAN`, and `dot intent apply PLAN --approve ID` interfaces.
  Read the generated exact plan and present direction, paths, additions/removals,
  permission and bounded integration effects before asking the owner to approve
  that specific ID. Never guess an approval token or reuse a stale plan.
- Unsafe/desktop effects and singleton skills-backup conflicts need their own
  acknowledgments. Cancellation or stale snapshots mean stop and re-plan, not
  force/reset/unscoped chezmoi application. Finish without changes is valid.
- Keep credentials, secrets, histories, runtime data, private inventories and
  recovery state out of output/public artifacts. Secret scanning is detection,
  not a guarantee. Do not run unapproved hooks or render credential-bearing files.

## Initialization and maintenance

- `dot init` owns dependency installs and tracker creation/detection. Each explicit
  install/create requires informed owner consent. Read-only detection does not
   silently create, repair, switch sources or widen adoption scope. Start with
   `dot init --check`; use guided `dot init` for independently consented setup.
   Init offers Finish by default, Packages Install missing (exact plan-ID approval)
   and optional prebuilt Track/Adopt rounds. Blanket `--yes` is not init consent.
- Dotfiles browsing, migration and `dot self-update` perform no package-manager operations.
  Packages Install missing is the browser's separately reviewed installation gate;
  list edits, navigation and deselection never install or uninstall.
  Public tooling maintenance is separate from approved private `dot pull` and
  application version-manifest updates (`dot update NAME VERSION`).
- Preserve private worktree/history, identity, exact local approval bytes and
  recovery state. Diagnose actual installed paths/config; deleting machine data
  is not routine repair. Public publication and live adoption need separate
  explicit authorization.
