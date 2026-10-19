# Folder browser verification — 2026-10-19

Status and the returning-user file tabs now browse immediate HOME-relative
children. Status groups tracked descendants into parent folder rows. Track
shows tracked and untracked HOME items together by default, greys untracked
entries, and opens their tracking options with Space. `f` optionally filters to
tracked paths only; HOME discovery remains lazy.
The selected item's status/diff occupies a narrow left column. Wide terminals
also show the selected folder's immediate children on the right.

Verified:

- A Status fixture with 720 skill files, two configs and a root file produces
  three HOME rows. Opening folders reveals one level; returning preserves focus.
- Track includes untracked siblings inside a partially tracked `.config`.
  The optional tracked-only filter preserves the selection basket and limits
  folder selection to exact tracked descendants.
- Space opens untracked file/folder options before selecting files. Cancellation
  leaves the basket unchanged. Saving folder options snapshots exact eligible
  files, maps their destinations relative to the chosen folder, and requires
  separate plan review. Late snapshot replies cannot change another tab's basket.
- Opening Track performs no recursive HOME directory expansion. Discovery loads the
  current folder only on request. Failed and cancelled loads remain retryable.
- Search spans the tracked catalog and returns to the current directory when
  cleared. Status inspection stays inline; preview scrolling uses its new width.
- Bounds checks cover 40×14 through 220×55 terminals. Rendered 80- and 140-column
  views, including grey untracked entries, were inspected. Another OS's physical
  variants are omitted.
- `go test -race ./...` and `go vet ./...` pass. The acceptance runner now
  reconstructs visible text across fragmented cursor/erase operations instead
  of treating incremental repaint fragments as complete screen contents.

The local `0.1.1-dev.3` package was cross-built and statically verified for
darwin-arm64, linux-amd64 and linux-arm64. The extracted Darwin package passed
the full keyboard/setup/Track/Adopt, selection/gates/recovery, offline and private
installation acceptance suite, including Space opening the tracking options,
canceling without selection, and explicitly saving options. The 10,000-file
browsing journey completed in 307.859625 ms, excluding fixture creation; this is
a local smoke measurement.

Artifacts and terminal transcripts remain under
`/private/tmp/dot-browser-20261019/accept-mixed-final`. The runnable preview is
`/private/tmp/dot-browser-20261019/preview-3/dot`. Its archive SHA256 is
`ffcc66a5357f1786105ade74a7a1c50b5506e803be5369bf70550598d1e99169`.
Linux packages were not executed. This preview does not publish a release or
replace the host's installed stable package.

## Full-height forms

The `0.1.1-dev.4` follow-up removes the metadata form's six-field display cap.
All ten fields appear when the workspace has room; short terminals retain a
window around the focused field. The forms were inspected at 80×24 and 140×45,
and `go test ./internal/tui` passed. All three platform archives were cross-built
and statically verified. The full keyboard gate above remains evidence for
`0.1.1-dev.3`; this display-only follow-up was not put through that gate again.
That follow-up's runnable preview is `/private/tmp/dot-browser-20261019/preview-4/dot`.

## Expandable status preview

In `0.1.1-dev.5`, Space in Tab 1 toggles the status/diff inspector between the
narrow left preview and the full workspace. Space or Esc collapses it without
changing the selected path, current directory, or filter. `J`/`K` scroll using
the expanded width. Space remains ordinary text while editing a search.

Race tests and vet pass for the TUI and acceptance packages. Expanded layouts
were inspected at 80 and 140 columns, with bounds checked down to 40 columns.
The extracted Darwin package passed the full keyboard acceptance suite, including
Space expanding and collapsing Tab 1. Transcripts remain under
`/private/tmp/dot-browser-20261019/accept-expand`.
That version's runnable preview is `/private/tmp/dot-browser-20261019/preview-5/dot`.

## Footer flow and changed folders

The `0.1.1-dev.6` display follow-up packs complete keybindings by terminal width,
ignoring early line breaks in hint definitions. Wide footers can use one line;
narrow ones keep the fixed two-row keymap and reserve Quit/Help when other hints
overflow. Folder names are yellow when any tracked descendant is changed,
missing, or unreadable. The color propagates through ancestors in both Status
and file-tab browsers, including child previews, while preserving selection
backgrounds and grey untracked entries.

The TUI tests and vet pass. Footer packing was checked at 40, 70 and 80 columns;
rendered layouts were inspected at 80, 140 and 220 columns. All platform archives
were cross-built and statically verified. The full keyboard gate remains the
`0.1.1-dev.5` run above. The latest runnable preview is
`/private/tmp/dot-browser-20261019/preview-6/dot`.

## Mouse and touchpad input

The `0.1.1-dev.7` preview enables terminal cell-motion/SGR mouse reporting.
Clicks focus rows and switch tabs, double-clicks open folders, and checkbox taps
use the existing selection/tracking-options actions. Scrolling targets the file
list, diff, or child preview under the pointer. Settings buttons, metadata
choices, setup choices, and footer key hints support clicks. Consent choices
only move focus; Enter remains a separate explicit activation. Mouse input calls
the same keyboard handlers and never calls policy/effect APIs directly.

Pointer regression tests cover wrapped tabs, scrolled-row hitboxes, child panes,
double-click boundaries, metadata save/cancel, Settings verification without
installation, independent review gates, final confirmation, and completion
navigation. Race tests and vet pass. The extracted Darwin package passed the
full acceptance suite, including real SGR press/release packets for tabs, search,
tracking options, option selection, cancellation, and preview expansion/collapse.
The journey checks that adoption remains empty and no source file is created.

Transcripts remain under `/private/tmp/dot-browser-20261019/accept-mouse-final`.
The latest runnable preview is `/private/tmp/dot-browser-20261019/preview-7/dot`.
All three platform archives were cross-built and statically verified; Linux
runtime input was not exercised.
