# T12 — Agents: approvals, activity, CLI contract (#34)

## Built
- `src/agent.rs`: one agent-identity rule (`DOTWARDEN_AGENT`, `AI_AGENT`, known agent
  environments); `changes::agent` and `pending::requester` delegate to it.
- Activity log `activity.jsonl` (command path, actor, exit code; no arguments), capped
  at 1000 lines; `dw activity [-n] [--actor]` (+ `--json`).
- `dw approve ID...` = interactive `pending apply`; agents get exit 3 and the item stays.
- `--dry-run` added to `fetch`, `pull`, `revert`, `machine use`, `approve`.
- `dw completions <shell>`, `dw man [--dir]` (clap_complete, clap_mangen), `dw skill
  [--install DIR] [--force]` printing/installing `skills/dotwarden/SKILL.md` (embedded).
- Spec §8/§9 updated.

## Decisions
- Known agent environments count only without a terminal: a human in an agent's
  terminal (this repo's owner runs OPENCODE=1) keeps TUI/approve rights. Explicit
  `DOTWARDEN_AGENT` always counts.
- Reads and `--dry-run` are not logged, so `status`/`doctor` stay write-free (existing test).
- `pull --dry-run` is offline: it reports from the last fetch.
- Skill install refuses to overwrite a differing file without `--force`.

## Tests
- tests/agents.rs, 4 end-to-end: known-env agent trailer/approval refusal/pending
  "requested by"; activity log content and no argument leakage; dry-runs leave refs,
  HEAD, commits and machine.toml unchanged; completions/man/skill and install safety.
- Hand-break: ignoring the terminal rule/known-env detection failed the agent test.
- fmt, clippy `-D warnings` and the full suite pass (macOS, isolated HOME/XDG, file backend).

## Gaps
- `vault init|unlock|lock|passwd|recover` have no `--dry-run` (passphrase flows).
- TUI does not show the activity log; it already marks "requested by" in pending.
- Man pages/completions are generated on demand, not shipped (T15 packaging).
- Not pushed; no GitHub changes.
