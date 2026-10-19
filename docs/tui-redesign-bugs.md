# TUI redesign: confirmed bugs and workflow gaps

Baseline: `20096ca5377f15572d597884875f26bce9193bd7` (2026-10-19).
Status: the Go baseline has been replaced by the accepted Rust/Ratatui design.
This register preserves the confirmed findings and stable IDs. The approved
contracts are in [the Rust spec](rust-rewrite-spec.md); current verification and
remaining platform limits are in [Rust acceptance](rust-acceptance.md). Historical
Go source references below resolve at the baseline commit rather than the new tree.

The original assessment is archived in
`.impeccable/critique/2026-10-19T14-13-16Z__internal-tui-model-go.md`.
Isolated temporary model/native fixtures reproduced findings as indicated below;
probe files were removed. Production HOME and private source were not changed.
The web-oriented design detector excludes Go and supplies no assurance here.

## Correctness defects

### BUG-11 — Symlinked installation misses bundled chezmoi (Rust regression)

- Trigger: invoke an extracted bundle through a separate `~/.local/bin/dot`
  symlink. On macOS, the executable path can retain that symlink location.
- Cause: dependency lookup used the unresolved executable parent, so it fell back
  to system chezmoi instead of the bundle's private binary.
- Fix: canonicalize the executable before locating `libexec/chezmoi`.
- Evidence: the installed-bundle check caught the fallback and rolled back before
  cleanup. A native symlink/version-directory integration test failed before the
  fix and passes afterward. Released in v0.2.1.

### BUG-10 — Startup file list stays empty until Left (Rust regression)

- Trigger: launch with existing HOME files and an unavailable source/failed native
  inspection, then dismiss the error. Files is empty until Left is pressed at HOME.
- Cause: the constructor left rows empty; only a successful snapshot or navigation
  rebuilt the local directory listing. Left at HOME inadvertently performed the
  missing initial scan.
- Fix: construct the HOME listing immediately, independently of native inspection.
  A successful snapshot subsequently adds managed/change metadata to those rows.
- Evidence: two startup UI tests failed before the fix, including the exact
  empty-before-Left/nonempty-after-Left comparison. The missing-source PTY fixture
  now checks that a real HOME file appears after dismissal without navigation.

### BUG-09 — Missing-source error reopens and prevents exit (Rust regression)

- Trigger: open the TUI with a nonexistent source, leave the error open beyond
  the 15-second refresh interval, then press Esc/Enter or try q.
- Cause: failed snapshots did not reset the refresh timer; dismissal immediately
  launched another failing refresh. Text modals swallowed q, and inspection jobs
  blocked quit using the same rule as mutations.
- Fix: failed inspection pauses periodic retries until an explicit refresh/setup;
  q exits informational screens and Ctrl+C is handled before modal/text input.
  Inspection/preview jobs permit exit; active operations still prevent interruption.
- Evidence: missing-source and quit UI regressions failed before the fix and pass
  afterward. `scripts/pty-missing-source.py` exercises the expired timer, both
  dismissal keys, q/Ctrl+C and terminal restoration against a real terminal.

### BUG-01 — Single-file Adopt hides replacement of the enrolled set (P1)

- Trigger: machine has several enrolled files; select one file in Adopt and apply.
- Actual: the selected set replaces enrollment. Omitted files remain on disk but
  lose future Update enrollment; omitted profile entries can become deselected.
  State changes are excluded from the review diff.
- Evidence: `internal/native/plan.go:252`, `:265`, `:363`;
  `internal/tui/model.go:1302`; `internal/tui/native.go:89`. Engine/control-flow review.
- Acceptance: a selected-file restore leaves enrollment unchanged; an intentional
  scope replacement explicitly lists enrolled/unenrolled files before approval.
  Cover existing customizations, profile members, and empty selection.

### BUG-02 — Selections change meaning across operation tabs (P1)

- Trigger: select in Adopt, preview, switch to Stop-Tracking, then preview again.
- Actual: the shared basket becomes an untrack request without selection in that
  operation; freshness remains true immediately after switching. Track selections
  may instead produce invalid Adopt requests because their items lack entry IDs.
  Navigation alone does not write; final confirmation remains.
- Evidence: `internal/tui/tabs.go:198`, `:258`; `internal/tui/model.go:1283`.
  Reproduced with a model fixture.
- Acceptance: selections retain their operation intent. Any explicit change of
  operation generates a new review and clears prior acknowledgments. Include
  keyboard/mouse navigation and delayed planning replies.

### BUG-03 — Existing tracked-file preview reverses Track direction (P1)

- Trigger: HOME differs from repository for a tracked file; inspect in Track.
- Actual: preview labels HOME → repository but shows local bytes removed and
  repository bytes added. The eventual Track plan shows the opposite diff.
- Evidence: `internal/native/inspection.go:336`, `:378`;
  `internal/native/track.go:276`. Reproduced with a native filesystem fixture.
- Acceptance: inline preview and final plan agree on before/after bytes and
  direction, including templates, variants, bounded Git fields and mode changes.

### BUG-04 — Status search consumes text as commands (P1)

- Trigger: press `/` in Status, then type `q`, `2`, or uppercase `W`.
- Actual: q quits, 2 opens Track, W opens Settings. Ordinary x enters the filter.
- Evidence: `internal/tui/returning.go:130`, `:142`, `:163`.
  Reproduced with key-event fixtures; a prior test used SetValue and missed it.
- Acceptance: focused input receives text before global character shortcuts.
  Exercise actual keys and pasted paths containing every command character.

### BUG-05 — Applying claims that no files are changing (P1)

- Trigger: confirm a plan whose application lasts long enough to render progress.
- Actual: Apply uses the generic loading state, which says “no files are changed.”
- Evidence: `internal/tui/model.go:1689`; `internal/tui/workspace.go:94`.
  Verified by control-flow review, not live production application.
- Acceptance: reading, planning, applying, committing and remote operations have
  accurate progress/results. Navigation and cancellation promises match reality.

### BUG-06 — Remote completion retains stale inventory and wrong page state (P2)

- Trigger: visit Adopt, then complete a Pull from Git Sync.
- Actual: cached inventory survives; stage changes to returning-home while active
  tab remains Git Sync, producing the legacy “What next?” page under that tab.
- Evidence: `internal/tui/model.go:272`; cached reuse in `internal/tui/tabs.go`.
  Reproduced with a remote-result model fixture.
- Acceptance: refresh affected inventory and invalidate stale derived plans;
  display a coherent completion state. Test newly pulled files and changed rules.

### BUG-07 — Advertised Packages shortcut is unavailable on Status (P2)

- Trigger: at width ≥90, the header says Packages (P); press P on Status.
- Actual: nothing happens. P has a different meaning in Git Sync (Push).
- Evidence: `internal/tui/workspace.go:70`; `internal/tui/model.go:597`, `:616`.
  Reproduced with a model fixture.
- Acceptance: visible shortcuts work in their displayed context without collision;
  hidden/unavailable actions are not advertised as executable.

### BUG-08 — Track filter footer is not clickable (P3)

- Trigger: click `f tracked only` or `f all HOME` in the footer.
- Actual: no action; keyboard f works because the pointer allowlist omits it.
- Evidence: `internal/tui/workspace.go:114`; `internal/tui/mouse.go:144`.
  Reproduced with footer hit-testing.
- Acceptance: all advertised clickable actions share keyboard/mouse semantics and
  remain correct after resizing and footer wrapping.

## Workflow gaps

### UX-01 — Changes cannot be resolved and published from their context (P2)

- Status provides inspection without selection/save/restore. Users must locate
  files again in another tab and translate their intention into Track or Adopt.
- Track changes the local checkout; Push publishes existing commits. There is no
  TUI commit step or actionable completion handoff, so Track → Push leaves the
  new edits unpublished. This is current documented policy, not a failed Git push.
- Evidence: `internal/tui/tabs.go:572`, `:512`; `internal/native/remote.go:99`.
- Acceptance: selected changes have explicit directional actions and a complete
  approved path to publication; unrelated edits/staged work are preserved.
  Distinguish HOME restoration from discarding repository changes.

### UX-02 — Git operations lack repository state (P2)

- Sync shows generic Pull/Push choices without actual branch, remote, working
  changes, staging, outgoing commits or useful divergence context.
- HOME matching the checkout is displayed as current, even if checkout edits
  remain uncommitted/unpublished. These are distinct states, not one clean flag.
- Evidence: `internal/tui/tabs.go:504`; `internal/tui/native.go:303`.
- Acceptance: show the relevant comparison and layer for each status; remote
  freshness must be explicit when offline or not fetched. Preview outgoing work.

### UX-03 — Source replacement stops at a plan (P2)

- Settings prepares a backup path, says “Original preserved,” and offers no
  continuation into exact-path confirmation/replacement. Planning creates no backup.
- Evidence: `internal/tui/tabs.go:398`; `internal/setup/source.go:239`, `:337`.
  Feedback/stage reproduced with a model fixture; planning code inspected.
- Acceptance: complete the approved replacement workflow or explicitly identify
  a plan-only operation with a working handoff. Report backup completion accurately.

## User-confirmed redesign requirement

### UX-04 — Layout fragments the product instead of exposing an accessible workspace

The user rejects the present layout even after the bugs are fixed. Their Lazygit
screenshot is a reference for persistent contextual panels, visible related state,
direct contextual actions, and a useful main details/diff area. Dot must present
Git, chezmoi and manifest behavior as one coherent experience. This supersedes
incremental polishing as the design direction. The accepted pane arrangement and
keys are recorded in the Rust spec.

Spec generation must cover each ID above, link it to acceptance cases, and record
any intentionally changed policy. Cross-platform build evidence must remain
distinct from Linux runtime validation.
