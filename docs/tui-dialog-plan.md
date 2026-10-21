# Action dialogs over the dot workspace

Status: implemented in the working tree. Review, progress, and results share a
dialog over the workspace; first-run setup/welcome remain entry screens.

## Intended behavior

Actions open a centered, bordered dialog over the existing workspace. The user
keeps the context of the selected files, folder, and panel throughout the action.
The user confirmed that review and result should remain in the same dialog.

An operation follows one dialog lifecycle:

`Preparing review → Review → Running → Result → Return to workspace`

Errors and partial results occupy that same dialog. Opening an action, preparing
its plan, and confirming it must not briefly dismiss the dialog or expose an
interactive workspace between phases.

## Current cause

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
- Keep first-run setup/welcome as dedicated entry screens. The source editor gets
  a large dialog with explicit expansion when more space is useful.
- Draw the workspace first, visually mute its background, then clear and render
  only the dialog rectangle. Workspace panels remain recognizable around it.
- Use compact content-sized dialogs for short forms and confirmations, a bounded
  scrollable dialog for menus/help, and a larger stable rectangle across all
  phases of an operation. Proposed operation maximum: 100 columns by 30 rows,
  leaving an outside margin when the terminal permits it.
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

## Implementation sequence

1. Add a small shared dialog layout/shell module, returning outer/content/footer
   rectangles and explicit clickable control bounds. Compute all bounds from the
   current terminal size on every draw.
2. Split workspace rendering from overlay rendering in `App::draw`. Remove the
   Review/Outcome early returns and the full-workspace clears for action dialogs.
3. Pass an explicit content rectangle into operation review/result rendering.
   Make wrapping, scroll limits, tab hitboxes, and buttons use that rectangle.
4. Represent preparing/running operation phases explicitly, preserving the
   dialog across worker completion, backend errors, and partial outcomes.
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
- Drive a synthetic operation through preparing, review, running, success, and
  failure: one stable dialog remains present; execution runs once only after
  explicit confirmation.
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
- Rust checks passed: 29 library tests, 7 filtering tests, and 32 workflow tests,
  including the background-error regression and the compact “Nothing to remove”
  message for untracked files/folders and mixed selections.
- Formatting, whitespace checks, and Clippy with warnings denied passed.
- Both PTY scripts passed, covering capture/ignore confirmation and results,
  search, resizing, startup/setup, exit keys, and terminal restoration.
- Rendered review, partial-failure result, and compact dialogs were inspected.
  Compact layouts reserve more space for preview content and stack controls
  when needed.
