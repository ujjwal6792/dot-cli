# T4 report (branch `t4`)

## Built

- `src/remote.rs`: remotes, visibility check, fetch, pull, push, pending-item planning. `src/changes.rs`: `apply_incoming`.
- CLI: `dw remote add|remove|list`, `fetch [--all]`, `pull [REMOTE]`, `apply [PATH…|--all] [--dry-run]`, `push [REMOTE|--all]`; `--json` everywhere, `--yes`/`--dry-run` where they mutate.
- Pending actions `remote_add`, `remote_remove`, `push` (all `needs_approval()`), guards = remote URLs + HEAD.
- TUI: background fetch on start and every 5 min (`FETCH_EVERY`, preference in T14), `f` fetch, `p` pull, `P` push (y/n); Incoming tab Space/Enter apply (y/n), `a` apply all; Conflicts tab refuses.
- Status ahead/behind now falls back to `<default remote>/<branch>` when no upstream is set.

## Decisions (please check)

1. **Visibility.** github.com, gitlab.com, codeberg.org, bitbucket.org: anonymous `git ls-remote <https-url> HEAD` (`-c credential.helper=`, `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=/bin/false`, 15 s timeout). Success = public (exit 6). Failure with a network sign (resolve/connect/timeout) = unreachable: warning, check skipped. Any other failure = not public.
2. **Local paths and `file://` remotes** count as private without confirmation (they cannot be public). Other hosts need a human "is it private?" answer (TTY prompt), stored per URL as `confirmed_private` in `machine.toml` (`write_local_machine` now preserves it). No TTY: exit 3 plus a pending item; approving that item in a terminal asks the question.
3. **Test hook.** `DOTWARDEN_VISIBILITY_PROBE=<cmd>` is run as `<cmd> <https-url>`: exit 0 public, 1 not public, else unreachable. It is read from the process environment, so it is not a security boundary (like agent approvals, ADR 0007).
4. **Approval.** Add, remove and push use the T3 `approval_gate`: agents, and humans without a terminal and without `--yes`, get a pending item (exit 3); a terminal asks y/N. `pending apply` by a human runs them, re-checking visibility again.
5. **Push** checks every target first, so one public remote stops all pushes before anything is sent. Sets upstream on first push. Never force.
6. **Pull** = fetch + `merge --ff-only`; diverged is refused (exit 1, "merge/rebase arrives later — use `G` lazygit"). Needs a repo with no unstaged edits.
7. **Apply** takes the lock, snapshots the Incoming files (reason `apply`), runs `chezmoi apply --keep-going` on exactly those targets without `--force` (a file edited meanwhile makes chezmoi refuse). Conflict named exactly: exit 5; with `--all` or a folder it is skipped and listed. Local files are never touched.
8. **Agents may apply Incoming files** (HOME equals last-applied, nothing is lost).

## Tests

`tests/remote.rs` (11, real git/chezmoi, local bare remotes, fake probe) + 2 in `tests/tui_changes.rs` (one replaces the T3 read-only test) + 1 unit test (https URL from ssh/scp forms).
Remote add/list/remove; public refusal with the probe seeing the https URL; private and unreachable known hosts; unverifiable host confirmation per URL; agent pending items and human approval of a push; public refusal before any push (also spares the private remote); unreachable probe warns; fetch leaves HOME and the repo alone; two machines: pull, Incoming, apply only incoming with snapshot; both edit → Conflict, apply refused (exit 5, `--all` skips); diverged pull refused; TUI apply prompt and `P`/`f`/`p`.
Hand-broken: visibility check removed from push (2 tests failed); apply given no targets plus `--force` (2 failed). Both reverted. fmt, clippy `-D warnings`, full suite pass. Help overlay text condensed to keep 24 rows.

## Gaps

- No override to apply over a Conflict or Local file (the ticket's "approval-required apply over local changes"): conflicts are only blocked until T9. Report if an explicit human override is wanted.
- `dw clone` registers the machine with a local commit. If another machine pushes before this one does, histories diverge and pull refuses until T11. The tests push the registration first.
- Remote add in the TUI and the privacy question there are not built (CLI/terminal only). Pending push via the TUI review works.
- Private HTTPS remotes get no credentials (git config is isolated); SSH works. Fetch/push run non-interactive (ssh `BatchMode`).
- Real network probe is untested (no network in tests); Linux not run.
