# Explicit daily Git operations belong in dot

The unified dot workspace owns staging, unstaging, explicit commits, fetch, pull,
push and history inspection so users can carry configuration edits through to
publication without leaving the application. An external-only Git handoff leaves
the main workflow incomplete; implementing the full set of advanced Git operations
would make this redesign unbounded. Advanced branch/stash/rebase/cherry-pick and
history-rewriting workflows therefore need a clear handoff initially.

This extends the previous pull/push-only surface. It preserves the distinction
between explicitly creating a commit and automatically committing as a side effect
of tracking, applying, setup, or pushing. The user accepted this scope during the
2026-10-19 design interview; specific Git/index semantics remain to be specified.
