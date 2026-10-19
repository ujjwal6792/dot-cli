# Dot workspace: consolidated design walkthroughs

Status: design review, not an implementation spec. Decisions Q1–Q22 are accepted;
Q23–Q25 were subsequently accepted and the user authorized the Rust rewrite.
Implementation evidence belongs in the Rust specification and acceptance notes.
See [discovery](tui-redesign-discovery.md) for answers and architectural sources,
and [the bug register](tui-redesign-bugs.md) for reproducible baseline defects.

## Workspace

```text
┌─ 1 Status ──────────────┬─ Details ──────────────────────────┐
│ Machine · branch       │ Diff · Source · Rules · Activity    │
│ HOME / source / Git    │                                     │
├─ 2 Files ───────────────┤ Selection drives this view          │
│ All · Managed · Changed│                                     │
│ ▸ .agents/             │ Compare: HOME → source              │
│ ▸ .config/             │         or source → HOME            │
│   .zshrc               │         or Git baseline → source    │
│                       │                                     │
├─ 3 Git ────────────────┤                                     │
│ Changes · staged      ├─ Activity ──────────────────────────┤
│ Commits · remote      │ Running / completed / failed        │
├─ 4 Machines ───────────┤ Expand for details and recovery     │
│ Mac · Linux           │                                     │
└───────────────────────┴─────────────────────────────────────┘
 Contextual actions · selected count                 ? Help
```

This shows information hierarchy, not fixed terminal dimensions. Files receives
most left-column height. Details and forms can expand to fill the screen. Narrow
terminals collapse inactive panels without making actions inaccessible. Footer
actions use available width and an accessible overflow/help route.

Each status states its comparison: HOME matching rendered source does not mean
the source is committed or published. Remote state has fetch/report freshness.
Grey untracked entries and yellow changed folders supplement text markers;
color alone is not the only indication of state.

## 1. Find a changed file without scrolling past 700 skill files

Open Files at HOME. `.agents/` and `.config/` are single collapsed directory rows.
Untracked entries remain visible in All, greyed out. A folder containing a changed
managed file is yellow and indicates changed descendants; a partially managed
folder does not imply all children are managed. Expand one directory at a time
or use Managed/Changed filters and search.

Typing `config2` or `sqlite` into search inserts text: it cannot change panels or
quit. Selecting a file updates Details while retaining position in the browser.
Mouse/trackpad events supported by the terminal invoke the same navigation and
actions. Touch support depends on the terminal translating gestures into events.

## 2. Publish two local edits

Select two changed files in Files, inspect the HOME → source comparison, and
choose Capture local changes. The affected destinations and source paths are
explicit. A directory selection resolves to an exact reviewable file set.

For ordinary files, chezmoi performs the import. For a rendered template, do not
overwrite the template with rendered bytes: use an established setting/source
mapping or resolve scope/source with the user. Shared versus machine scope is
visible before changing source. Preview and operation use the same direction.

After capture, the UI reports source changes and offers the next Git action.
Stage desired source files in Git, review everything actually staged (including
work staged externally), enter a commit message, commit, then explicitly push.
Capture alone never claims publication. Whole-file staging is in scope; hunk
staging and advanced Git history manipulation are deferred.

## 3. Reject an unwanted edit

If HOME has an unwanted edit, Restore local file previews rendered source → HOME
and offers recovery for overwritten local content. If repository source has the
unwanted edit, Discard repository changes names its Git baseline and affected
source paths. These are different actions with different destinations.

Restoring one file must not remove other files from the machine's update scope.
Selection does not silently acquire destructive meaning when focus changes.
An explicit new operation produces a fresh operation-specific review.

## 4. Change a shared Ghostty setting

For a supported mapped setting, Details shows its effective value, origin and
known inheriting machines. Change shared font size 14 → 16, commit and push.
Mac and Linux inherit 16 unless an OS or machine override takes precedence.
“Override on this machine” creates the specific exception; removing that override
returns to inheritance. Existing arbitrary templates retain their semantics and
may only offer source editing rather than structured setting controls.

Linux's opt-in worker checks for updates while awake, normally every five minutes.
It pauses on dirty source/HOME conflicts or Git divergence. Strict mode applies
only eligible updates; unproven dynamic behavior waits for review. Explicit local
Trusted repository mode permits unattended native behavior with broader possible
effects. The remote repository cannot silently grant itself this trust.

The Mac sees Linux's last report, not a live assertion that the setting is active.
A hypothetical render for Linux is labeled separately from Linux's observed state.

## 5. Open a fresh machine or an ordinary chezmoi repository

Open and inspect an ordinary repository without a mandatory dot manifest. Missing
dot metadata means a relationship or activity history is unavailable, not that it
is empty or healthy. Explicit manual chezmoi operations remain available.

**Q23 accepted:** local automatic-update enablement is separate from source
tracking. Enable selected configurations on this machine incrementally; disable
them without deleting HOME content or forgetting shared source. Apply/Restore
does not replace that set. Stop tracking edits shared source and explains its
cross-machine consequence if committed/pushed. New targets need explicit review
before entering strict automatic scope.

## 6. Observe user and agent work

The compact Activity area shows current and recent operations. Expand it to see
available actor, intent, paths, timestamps and outcomes. External edits without
reliable attribution remain unknown; a Git author is not proof of who changed HOME.
Records can be running, successful, failed or partial. Local retention defaults to
30 days and is configurable; config values and raw output are not logged by default.

Init installs the small versioned `.agents/skills/dot/SKILL.md` in HOME, preserving
conflicting files and user edits. It describes implemented commands. Ordinary
direct chezmoi/Git work remains supported, with richer activity available when
agents use dot's structured interface.

Per-machine report publishing is a separate opt-in. Summary report branches carry
revision/time/outcome/counts, and can be automatically committed/pushed without
staging or publishing configuration changes. Detailed activity stays local by
default. A failed report upload does not relabel a successful apply as failed.

## 7. Handle an interrupted or partial operation

Progress distinguishes inspecting, capturing, applying, committing and publishing.
It never states that no files change during application. When three files succeed
and a fourth fails, show updated/failed/not-attempted results and retain available
backups. Refresh before retry; explicit restore is offered where possible.
Neither arbitrary script effects nor multi-file chezmoi operations are presented
as a universally atomic transaction.

**Q24 accepted:** serialize dot-owned writes across TUI, CLI and background sync.
An external change to reviewed files, rules or Git baselines invalidates the
review. Refresh and request a new decision rather than applying a stale destructive
operation. External processes may bypass dot's coordination; detecting changes
does not make arbitrary templates deterministic or eliminate every race.

## 8. Pull incoming changes or meet a conflict

Git displays branch, staged/unstaged source changes, outgoing commits, remote
freshness and divergence. Pull fast-forwards only when clean; it does not silently
stash or rebase. Conflicts/divergence pause with useful diagnostics and a handoff
to Git tools for advanced resolution. Returning refreshes the actual state.

After a successful pull, invalidate affected inventory and reviews. Newly added
files and changed rules appear in the existing workspace without jumping to the
legacy home screen. The next Apply remains distinct from publication and from
automatic local enablement.

## 9. Migrate the old dot catalog

Explicitly start migration and inspect mapped and unresolved entries. Preserve
original source/config/local state in a recoverable archive before conversion.
Keep HOME unchanged until a separate Apply. Dot-specific template data, partial
Git configuration management and precedence differences require deliberate
conversion; archive preservation alone is not equivalent behavior.

Do not infer another machine's enrollment from this machine's local state.
Ambiguous cases remain unresolved with their originals recoverable. Migration
does not run silently merely because the user opened the new application.

## 10. Reach utilities without losing workspace context

**Q25 accepted:** Settings, existing Packages support, recovery and source switching
live in the global action menu and contextual help. They are reachable from every
panel with working advertised actions; they do not each need a permanent panel.
Source replacement completes preview → archive → replacement → result, with an
honest recovery path. A proposed backup path is never labeled “Original preserved”
until the backup actually exists.

## Bug-to-walkthrough coverage

All issues remain open until implementation and verification. This is design
traceability, not test evidence that the defects have been fixed.

| Issue | Walkthrough and acceptance focus |
|---|---|
| BUG-01 | 3, 5: selected restore preserves unrelated enablement |
| BUG-02 | 2, 3, 7: operation-specific selection, fresh review |
| BUG-03 | 2, 3: comparison direction matches actual operation |
| BUG-04 | 1: actual typing takes precedence over command shortcuts |
| BUG-05 | 7: operation-specific progress and partial results |
| BUG-06 | 8: invalidate stale state and stay in coherent workspace |
| BUG-07 | 10: advertised utilities are reachable without collisions |
| BUG-08 | 1: pointer and keyboard share action definitions |
| UX-01 | 2, 3: select → capture/restore → stage → commit → push |
| UX-02 | 2, 4, 8: HOME, source, Git and remote freshness are distinct |
| UX-03 | 9, 10: completed archive/replacement with truthful feedback |
| UX-04 | Workspace and 1–10: persistent panels and contextual actions |

The spec will turn these flows into concrete state transitions, command contracts,
key bindings, responsive layouts, migration cases and testable acceptance criteria.
Mac runtime, Linux runtime and cross-compilation evidence must remain distinct.
