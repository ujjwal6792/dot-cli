---
target: dot TUI workflows and change management
total_score: 14
max_score: 40
na_heuristics:
p0_count: 0
p1_count: 5
timestamp: 2026-10-19T14-13-16Z
slug: internal-tui-model-go
---
Method: dual-agent (A: /root/ux_workflows · B: /root/ux_evidence), with parent fixture verification.

Reviewed current TUI at commit 20096ca5377f15572d597884875f26bce9193bd7. This is a workflow critique, not a review against a historical diff. The terminal browser has a coherent visual identity and useful folder grouping, but its operation model exposes engine modes instead of completing the user's change-management tasks.

## Design health

Scores are qualitative UX judgments, not automated measurements (0–4 each).

| Heuristic | Score | Evidence |
|---|---:|---|
| Visibility of system status | 1 | Applying claims no files are changed; Git state is absent |
| Match with user tasks | 1 | Track/Adopt do not map cleanly to save/restore/publish |
| User control and freedom | 1 | No contextual restore or publish path |
| Consistency | 1 | Shared baskets change meaning; advertised shortcuts fail |
| Error prevention | 2 | Exact plan gates exist, but enrollment consequences are hidden |
| Recognition over recall | 1 | Must find files again and know operation terminology |
| Efficiency | 2 | Folder browsing works; changed-only workflow missing |
| Minimalist design | 2 | Compact browser; metadata and policy language still dominate |
| Error recovery | 1 | Dirty checkout and recovery send users outside the workflow |
| Help/documentation | 2 | Key help exists; some advertised actions are unavailable |
| Total | 14/40 | Significant workflow and interaction defects |

## What works

- Immediate-child folder browsing makes large skill/config catalogs navigable.
- Exact-file plans, independent acknowledgment gates, and Cancel-default confirmation provide useful effect boundaries.
- Mouse support, keyboard navigation, grey untracked rows, and changed-folder colors establish a useful browsing foundation.

## Priority findings

1. **P1: Adopt can silently replace the whole enrolled set when the user selects one file.** `internal/native/plan.go:252` replaces `next.Entries` with `p.Selected`; omitted profile entries become deselected at line 265. Other HOME files remain, but disappear from future Update enrollment. `PlanDiffs` omits state actions at line 363, and `internal/tui/native.go:89` supplies no enrollment delta. Restore selected files without altering scope; present scope replacement separately with explicit additions/removals. Suggested follow-up: shape the restore/enrollment flows, then harden their reviews.

2. **P1: Selection intent leaks between operation tabs.** `internal/tui/tabs.go:198` preserves a single basket while switching `m.mode`; `internal/tui/model.go:1283` builds requests from that mode. Reproduced: select in Adopt, switch to Stop-Tracking, press p; the selected IDs now form an untrack plan without any selection in that tab. Confirmation still prevents immediate writes. Use independent per-operation selections/plans and explicit transfers. Suggested follow-up: harden operation state.

3. **P1: Track's inline diff is reversed for existing tracked files.** `internal/native/inspection.go:336` labels HOME → repository, but line 378 builds a HOME action from repository content. Native fixture: cursor preview removes local content/adds repository content; the eventual Track plan does the reverse. Generate cursor and final previews from matching directional actions. Suggested follow-up: harden preview correctness.

4. **P1: Status search executes application shortcuts.** `internal/tui/returning.go:130` handles q and lines 142–154 handle tab keys before search at line 163. Reproduced: / then q quits; 2 opens Track; W opens Settings. Route focused input before global character shortcuts. Suggested follow-up: harden input ownership.

5. **P1: Applying tells the user no files are changing.** `internal/tui/model.go:1689` sets loading while executing Apply; `internal/tui/workspace.go:94` renders “Loading… Navigation is available; no files are changed.” Distinguish reading, planning, applying, and remote operation states and report the active effect accurately. Suggested follow-up: clarify operation feedback.

6. **P2: Status cannot resolve the changes it identifies, and Track → Push does not publish them.** `internal/tui/tabs.go:572` only supports browsing/preview actions. Track saves into the local checkout; `internal/native/remote.go:99` explicitly pushes only existing commits. There is no TUI commit step or selection-preserving handoff. Keep read-only browsing, but add selection plus explicit Save to repository, Restore HOME from repository, and reviewed Commit/Push actions. Preserve Space's requested expand/collapse behavior; provide a different selection key and clickable checkboxes. Suggested follow-up: shape the complete change workflow.

7. **P2: Git Sync offers operations without repository context.** `internal/tui/tabs.go:504` displays generic Pull/Push choices and command templates, without actual branch, origin, pending working changes, staged files, or outgoing commits. HOME-versus-source equality does not mean changes were committed or published. Show those states independently; distinguish Restore HOME from repository from Discard repository edits since HEAD. Suggested follow-up: shape Git Sync information architecture.

8. **P2: Pull completion leaves stale inventory and inconsistent navigation.** `internal/tui/model.go:272` sets stage to returning-home but retains activeTab=GitSync and invCache. Reproduced via fixture: legacy “What next?” content under Git Sync and cached old Adopt inventory after successful pull. Invalidate affected inventories/plans and land on a coherent result page with refresh/next actions. Suggested follow-up: harden remote completion.

9. **P2: Settings replacement stops at a plan with misleading completion language.** `internal/tui/tabs.go:398` stores a backup path and says “Original preserved,” but never opens confirmation or stores the plan for replacement. `internal/setup/source.go:337` only calculates the backup plan. Complete the review/confirm/replace flow or label this as a plan-only diagnostic and provide an actionable handoff. Suggested follow-up: clarify and complete source maintenance.

10. **P2: Packages shortcut is advertised where it does not work.** `internal/tui/workspace.go:70` shows Packages (P), but returning-stage routing in `internal/tui/model.go:597` intercepts Status before the package handler at line 616. Fixture confirms P does nothing on the landing screen; in Git Sync it means Push. Scope the shortcut correctly or make it consistently available. Suggested follow-up: harden command routing.

11. **P3: The Track filter footer cannot be clicked.** Visible `f tracked only`/`f all HOME` is omitted from the mouse allowlist at `internal/tui/mouse.go:144`. Fixture hit-test returns no action. Render hints and dispatch from one shared action definition. Suggested follow-up: harden mouse affordances.

## Cognitive load and user journeys

The main problem is translating intent into engine operations: “keep my edit” requires Track, review, acknowledgments, confirmation, an external Git commit, then Git Sync. “Discard my edit” suggests Adopt but can replace enrollment. Six operation tabs and ten metadata fields expose many decisions before the common action is clear. Prefer contextual verbs and disclose advanced metadata when relevant.

For a daily user, discovering a changed file is followed by rediscovery in another tab and an external commit step. For a fresh-machine user, selecting a few files looks additive but actually replaces the enrolled set. For a keyboard user, search text can leave the task or quit. Confidence drops at the review screen when the diff direction is wrong or scope changes are omitted, and completion does not show what remains to publish.

## Recommended product behavior

Make Status a change-management workspace: changed-only/all filters, exact file/folder selection, and visible selected count. Preserve Space expansion. Offer explicit HOME → repository save and repository → HOME restore with the same directional preview and final review. Keep repository working changes, commits, and remote publication distinct; offer an explicit commit message and selected-change commit, followed by push of the reviewed outgoing commits. Preserve unrelated staged/working changes. Show enrollment separately from file synchronization and show its delta in review.

Fix P1 correctness first, then connect the Status → save/restore → commit/push workflow. The core design question is which selected files and which direction the user is authorizing, not which internal mode they must learn.

## Evidence limits

Two independent assessments plus parent fixture checks. Temporary model/native tests reproduced cross-tab intent, reversed preview, Status search, Packages, mouse footer, remote completion, and backup-plan feedback; all probe files were removed. Apply feedback and scope replacement were traced through engine/rendering code. No production HOME or private dotfiles changes were applied.

The required detector ran successfully and returned zero findings, but its extensions exclude Go: this is no automated visual assurance. The target is a native TUI; no browser, overlay, or server was used. Real terminal mouse delivery and color rendering were not re-exercised during this review. Existing screenshots and source/rendered fixture behavior supplied the UI evidence.
