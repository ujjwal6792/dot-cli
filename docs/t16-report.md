# T16 — Backup workflow fixes

## Built
- `track`/`untrack` with several paths (`track::plan_paths` → `Planned{items, skipped}`,
  `cmds::submit_skipping`): valid paths planned, refused ones listed under "Skipped:" /
  `skipped` in `--json`, `--dry-run` and `--apply`. A single path, or none plannable, errors as before.
- `clone` (`cli::share_registration`): a human pushes "Register machine" to `origin` right
  away after `ensure_not_public`; otherwise prints `run dw push to share this machine with the
  others` (exit 0; `--json` gets `pushed`, `push_note`, `hint`). Agents never push.
- Push satisfies pending pushes (`remote::drop_satisfied`, `forget_satisfied`): `dw push`,
  TUI push and `pending apply` of a push item remove same-branch items whose remotes were all
  pushed (empty remotes = all) and say so (`pending_removed` in `--json`). `Action::Push`
  gained `branch` (`serde(default)`, so old pending files load).
- `apply` (`changes::apply_incoming`): `replaced_existing` = applied files present in HOME with
  no chezmoi last-written record. Listed with snapshot id and `dw snapshot restore <id>`;
  up front in `--dry-run`; in `--json`; in the TUI Incoming apply message.
- HTTPS (`remote::net_run`, `ssh_url`, `is_auth_failure`, `https_warning`): fetch/pull/push
  auth failures add the SSH URL; `remote add` warns (not refuses) for known hosts.
- Skill `skills/dotwarden/SKILL.md` updated (partial track, clone hint, pending push).

## Decisions
- Clone treats its origin like `remote add`: local paths are never public (no prompt); known
  hosts use the anonymous probe; unverifiable hosts ask on the terminal (stored as confirmed
  private) and without a terminal stay local with the hint. Probe offline: warning, push tried.
- Satisfaction needs every remote the item names covered by the push.
- `untrack` shares the partial-success rule (same planner).
- Spec §5/§6 already described all of this; no spec change, no deviation.

## Tests
- `tests/backup.rs` (new, 5 end-to-end): partial track (`--dry-run`, `--json`, `--apply`,
  all-refused, single path); two-machine clone (A pulls B's registration without a push; agent
  clone stays local with the hint); pending push removed by `push --yes`; apply listing replaced
  `~/.zshrc` (dry run, snapshot hint, not repeated); `remote add` HTTPS warning.
- Unit: `ssh_url`, `https_warning`, `is_auth_failure` on sample git stderr (no network).
- Updated `tests/track.rs` (old all-or-nothing test) and `tests/remote.rs` (B's registration is
  already on the remote after clone, so A pulls first).
- Hand-broken once each: clone push, pending removal, replaced-files filter, partial track — all failed a test.
- Gates: fmt, clippy `-D warnings`, full `cargo test` green.

## Gaps
- The TUI apply confirmation dialog does not list replaced files up front, only the result message.
- A pending push item that another push satisfies is removed only through `dw push`, TUI push and
  `pending apply`
- The auth-failure path (net_run) has only unit coverage; no HTTPS server in tests.
- Not pushed; no GitHub changes.
