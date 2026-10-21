# Action dialogs over the dot workspace

Status: implemented in the working tree. Review, progress, and results share a
dialog over the workspace. Preparing a review and other background loading use
toasts at the bottom right; first-run setup/welcome remain entry screens.

## Intended behavior

Actions open a centered, bordered dialog over the existing workspace. The user
keeps the context of the selected files, folder, and panel throughout the action.
The user confirmed that review and result should remain in the same dialog.

An operation follows this lifecycle:

`Preparing review (toast) → Review → Running → Result → Return to workspace`

Errors and partial results occupy the operation dialog. Preparation keeps the
workspace interactive. A ready review waits for an active search/form to close,
so typing cannot accidentally approve an operation. Once the review opens, its
dialog stays present through confirmation, execution, and results.

## Original cause

- `App::draw` in `src/ui.rs` renders Review and Outcome before the workspace and
  returns immediately, so those views replace it entirely.
- The remaining modal variants clear `outer[0]`, the full workspace content area.
- `src/operation_screen.rs` obtains `f.area()` internally and paints the whole
  terminal before centering its content. Centered content alone is not an overlay.
- Starting an action clears `self.modal`; confirming a review also removes it
  while the worker runs. A geometry-only change would leave this interruption.

## Scope and interaction rules

- Convert action menus, short inputs, confirmations, operation reviews/results,
  help, and errors to dialogs with one consistent shell. Keep existing colors,
  action semantics, exact-target previews, and explicit confirmation keys.
  Enter and `y` both confirm operation reviews and local action confirmations.
- Keep first-run setup/welcome as dedicated entry screens. The source editor gets
  a large dialog with explicit expansion when more space is useful.
- Draw the workspace first, visually mute its background, then clear and render
  only the dialog rectangle. Workspace panels remain recognizable around it.
- Use compact content-sized dialogs for short forms and confirmations, a bounded
  scrollable dialog for menus/help, and a larger stable rectangle across all
  dialog phases of an operation. Operation maximum: 100 columns by 30 rows,
  leaving an outside margin when the terminal permits it.
- Preparation is an immediate bottom-right toast with an Esc/click cancel control.
  Background inspection and detail-loading toasts appear only after 250 ms to
  avoid flashes. Every visible toast stays for at least 0.8 seconds before the
  result is presented, then disappears automatically. Cancellation bypasses this
  minimum. The timer starts at first render and never sleeps the UI thread. Toasts
  never dim the workspace or take keyboard focus; their bounds adapt on resize.
- On narrow terminals, use the available space with a minimal margin; on very
  small terminals, allow the dialog to fill the viewport. Keep confirmation,
  cancel/close controls, and scrolling usable. Never require a larger terminal
  just to cancel an operation review or read a failure.
- Keyboard and mouse input belong to the active dialog. Ignore clicks outside
  it, including clicks on underlying panel tabs and actions. Scroll only dialog
  content; background cursor, focus, and selections must not move.
- Esc cancels preparation/review or closes a completed result. During a mutation,
  show progress and disable repeat confirmation and dismissal; do not imply that
  Esc cancels backend work. Ignore results from abandoned preparation jobs.
- Closing returns to the same panel, directory, filter, search, and nearest valid
  cursor position. Preserve established action selection cleanup after success.
  Only an explicit “Review in Git” action changes the panel.
- Files shortcuts: `r` removes saved copies and keeps originals; `R` restores
  originals from saved configuration; `d`/Delete moves untracked originals to
  system Trash after exact-path review. Ctrl+R refreshes. Trash refuses managed
  files, folders with managed descendants, and dot source/state/Git internals.
  A folder review includes all its contents; changed contents invalidate the
  plan. Trash failures stop the batch and report remaining items. Recovery uses
  system Trash, without a permanent-delete fallback.

## Implementation sequence

1. Add a small shared dialog layout/shell module, returning outer/content/footer
   rectangles and explicit clickable control bounds. Compute all bounds from the
   current terminal size on every draw.
2. Split workspace rendering from overlay rendering in `App::draw`. Remove the
   Review/Outcome early returns and the full-workspace clears for action dialogs.
3. Pass an explicit content rectangle into operation review/result rendering.
   Make wrapping, scroll limits, tab hitboxes, and buttons use that rectangle.
4. Represent preparation as a toast and execution as a dialog, preserving the
   dialog across confirmation, backend errors, and partial outcomes.
5. Move the other action modal renderers into the shared shell. Replace mouse
   coordinate guesses with actual control rectangles and enforce focus isolation.
6. Update relevant README guidance and existing render/interaction acceptance
   checks to describe dialogs rather than dedicated action screens.

Backend planning, execution, recovery, CLI output, and Git/chezmoi behavior are
outside this change.

## Acceptance checks

- Render at 120×35 and 80×24: visible workspace surrounds review and result;
  a bordered dialog contains the entire action UI.
- Render at 45×14 and 20×8: no panic, reachable primary/cancel controls, and
  scrollable overflow. Resize while reviewing, running, and reading a result.
- Drive an operation through preparing, review, running, success, and failure:
  preparation uses a toast; review, execution, and result use one stable dialog.
  Execution runs once only after explicit confirmation. A ready review cannot
  interrupt typing in search or another form.
- Click and scroll outside the dialog: no underlying action, focus change, or
  list movement. Verify real dialog button and tab hitboxes after resizing.
- Close or cancel: return to the original navigation context. Verify the explicit
  Git shortcut still goes to Git.
- Keep every exact target and long preview reachable, plus partial-failure,
  skipped-file, and raw-details information.
- Use Ratatui TestBackend fixtures for rendering/input checks without requiring
  chezmoi. Then run existing backend tests and PTY smoke checks with chezmoi
  available; those integration checks remain necessary before calling it fixed.

## Verification completed

- The overlay regression initially failed because the action menu occupied the
  workspace; it now passes for menus, forms, confirmations, help, reviews, and results.
- Rust checks passed: 47 library tests, 7 filtering tests, and 32 workflow tests,
  including the background-error regression and the compact “Nothing to remove”
  message for untracked files/folders and mixed selections.
- Tree-sitter checks cover bundled grammars, multiline/Unicode source, editor
  caret placement, source/preview envelopes, both sides of multi-file diffs,
  cache invalidation, readable fallbacks, and colors in actual detail/editor/review
  rendering. Review syntax colors were visually checked at 120×35; existing
  fixtures also pass at 80×24, 45×14, and 20×8.
- Toast checks cover placement, unchanged workspace rendering, keyboard/mouse
  cancellation, resizing, delayed background status, automatic dismissal, and
  deferring ready reviews until search or another form has closed.
- Trash checks cover exact approval, untracked-only validation, protected paths,
  nested folder freshness, symlink targets, partial failures, and recovery guidance.
  A generated temporary folder was also moved through native macOS Trash and
  restored; its external symlink target was preserved. Terminal checks cover
  `d` review/cancellation and `r` removal of saved copies.
- Formatting, whitespace checks, and Clippy with warnings denied passed.
- Both PTY scripts passed, covering capture/ignore confirmation and results,
  search, resizing, startup/setup, exit keys, and terminal restoration.
- Rendered review, partial-failure result, and compact dialogs were inspected.
  Compact layouts reserve more space for preview content and stack controls
  when needed.
