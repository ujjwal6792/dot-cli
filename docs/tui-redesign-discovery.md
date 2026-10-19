# Unified dot workspace: design interview

Status: Q1–Q25 accepted; user authorized a Rust + Ratatui implementation rewrite.
Started: 2026-10-19. Baseline: `20096ca5377f15572d597884875f26bce9193bd7`.
Workflow: user-invoked grill-with-docs (grilling + domain-modeling).

## Accepted user direction

- The previous audit findings are valid; preserve them for spec generation in
  [the bug register](tui-redesign-bugs.md).
- The current layout is insufficient; redesign the interaction model and layout,
  beyond isolated fixes and spacing changes.
- Lazygit's supplied screenshot is the interaction/layout reference: accessible
  persistent panels and contextual actions with a substantial detail/diff surface.
- Dot handles Git, chezmoi and manifest together as one product experience.
- Chezmoi does the file-management heavy lifting. Dot provides the TUI and lets
  users see how agents manage their dotfiles across different machines. This
  supersedes maintaining dot's independent import/render/diff/apply implementation.
- Work through the design interview and record settled terminology/decisions;
  generate a spec after shared understanding is confirmed.

## Existing facts versus open choices

Existing code separates HOME, repository source, Git commits/remotes, and local
enrollment. Manifest assignment controls eligibility; enrollment authorizes the
machine's exact scope. Track imports HOME into source; Adopt currently replaces
enrollment and applies source to HOME. Git Push sends existing commits only.
These facts explain current behavior; they do not settle the redesigned UX.

Fact investigation confirmed that explicit Commit is new functionality, but is not
forbidden by the existing prohibition on automatic commits. Staging, fetch,
history, branches, stash and conflict tools are also new scope. The bundled chezmoi
is pinned/verified and supplies tracker-data compatibility; current configuration
application and templates are handled by the native Go policy engine. Full native
chezmoi behavior would be a substantive capability change, not just new panels.

The current glossary defines Adopt as enrollment, which is not the same intent as
restoring one file. Q15 separates directional file actions. Q23 below settles the
remaining local enrollment controls. Avoid a generic “discard” spanning HOME and
repository edits without identifying its destination/baseline.

The accepted interaction direction replaces the prior narrow-left-preview
arrangement with persistent left panels and a substantial right details pane.
Space becomes contextual selection/staging, with a separate expansion key.
Panel contents are accepted in Q11; proportions and key assignments remain to be designed.

## Decision tree

### Round 1: accepted decisions

1. **Accepted: primary object.** One logical configuration file in a familiar folder
   hierarchy, with HOME/source/Git/rules together and expandable variants/physical
   files. Separate HOME and repository browsers are not the primary organization.
2. **Accepted: Git scope.** Stage/unstage, explicit commit, fetch, pull, push and
   history inside dot. Full branch/stash/rebase/cherry-pick/history-rewriting tools
   are outside this redesign's initial scope; unsupported situations need a handoff.
3. **Accepted: action cadence.** Reversible actions with recovery and direct
   feedback; destructive operations get contextual preview/confirmation. Commit
   uses a message form and Push remains explicit. Exact validation/freshness stay;
   repeated UI gates for every write are not the desired interaction.
4. **Accepted: interaction conventions.** Adopt Lazygit-style panel focus and
   contextual actions, including replacing earlier controls. Numbers focus panels,
   Enter enters detail, Esc returns, Space selects/stages as labeled, expansion has
   a separate key. Final keymap still requires collision and context review.
5. **Accepted: chezmoi owns file-management behavior.** Delegate import, rendering,
   comparison and application to chezmoi. Dot supplies the unified TUI, Git workflow
   and visibility into user/agent activity across machines. Keep the integration
   aligned with chezmoi semantics rather than rebuilding them. See ADR 0006.

### Round 2: decisions and remaining frontier

6. **Accepted: repository compatibility and metadata.** Ordinary chezmoi
   repositories work immediately, with optional dot metadata for enhanced
   relationships/activity/catalog details. Standard chezmoi source is authoritative;
   a mandatory dot manifest is not a prerequisite for basic use. See ADR 0007.
7. **Accepted: agent integration.** Support ordinary chezmoi/Git/filesystem work
   and optional structured dot integration. Recorded actions carry available
   intent/actor information; unexplained external changes remain unknown.
   User addition: `dot init` installs a small skill under `.agents/` that introduces
   dot and guides agents to its interface. Q20 settles the path and preservation
   policy; exact content and event schema belong in the spec. The skill must
   describe the implemented CLI rather than invent commands.
8. **Accepted through Q12: cross-machine scope.** Show intended configuration and
   timestamped machine reports. Each machine executes its own local operations;
   remote shell/execution controls are outside the initial redesign.

### Shared configuration scenario raised during round 2

User asks for one configuration composed of common lines plus machine-specific
lines, so a common Ghostty font-size change on the Mac can propagate to Linux.
Shared configuration and opt-in background pull/application are accepted below.
Q12 subsequently settled Q8's remote-execution boundary.

- Chezmoi can render shared templates/data with conditional machine content;
  dot need not implement an arbitrary line-patch engine. Sources:
  [machine differences](https://www.chezmoi.io/user-guide/manage-machine-to-machine-differences/),
  [templating](https://www.chezmoi.io/user-guide/templating/).
- Accepted semantics: a shared value is inherited unless an explicit override
  applies. Resolve the effective value in the template; simply appending duplicate
  configuration lines is not universally a valid override mechanism.
- Example: shared font size 14→16 reaches both Mac and Linux if neither overrides
  it. A Mac-only override to 16 leaves Linux at shared 14. An existing Linux-only
  override must remain visible instead of suggesting the shared change took effect.
- Editing a rendered HOME file does not reveal which source/template/data scope
  the user intended. Chezmoi re-add deliberately does not overwrite templates:
  [re-add](https://www.chezmoi.io/reference/commands/re-add/).
  Shared-value editing can update a known source location; arbitrary external
  edits require a source-edit/mapping workflow, not a promise of automatic inversion.
- Publication and application are separate: Mac edits shared source → commit/push
  → Linux pulls source → Linux renders/applies. Automatic pull/application needs an
  explicit per-machine policy and handling for dirty files, divergence and failures.

9. **Accepted: edits follow declared scope.** Editing a known shared setting changes
   its shared source and affects inheriting machines. Provide an explicit override
   for this machine. Unmapped external HOME edits require resolving source and
   scope; do not infer a generic inverse transformation of arbitrary templates.
10. **Accepted: opt-in automatic pull and apply per machine.** A machine may
    automatically pull and apply configurations enabled on it. Pause for local
    edits or conflicts. This does not authorize automatic commit/push, remote
    execution, or installation of a background service during this interview.
    Q12 and Q14 settle the service lifecycle and incoming-change review policy;
    enforcement and error handling still require concrete design.

### Round 3: accepted workspace and machine behavior

11. **Accepted: workspace layout.** Persistent left panels for Status, Files, Git and
    Machines; large right details with contextual Diff/Source/Rules/Activity views;
    a compact expandable activity log below. Files get most of the left-column
    height. Panels can expand; forms can occupy the full screen.
12. **Accepted: machine execution boundary (resolves Q8).** Each machine runs its own opt-in
    background synchronization and reports dated results, including when the TUI
    is closed. No remote shell/execution controls in the initial redesign. Q19
    settles the default cadence and report transport.
13. **Accepted: inheritance.** Shared defaults → OS-specific overrides → machine-specific
    overrides for explicitly modeled settings, rendered through chezmoi. Arbitrary
    existing templates retain their own semantics. Show effective value and origin.
14. **Accepted: automatic application policy.** Eligible existing configurations may update
    unattended; newly enabled destinations, deletions, and executable changes require
    review. Templates/hooks/scripts make enforcement a substantive integration
    question: feasibility must be checked before promising this boundary. If a
    repository cannot be evaluated under that policy, pause automation for review.

### Feasibility findings after round 3

Chezmoi supports explicit apply targets and nonrecursive selection, but target
selection does not constrain arbitrary code executed by templates, hooks, modify
scripts or externals. Hooks run even during dry runs. Empty rendered templates and
exact directories can delete destinations; filtering script/remove entry types is
insufficient to enforce Q14. Preview and apply are separate evaluations, not a
sealed transaction. Q16 subsequently settled the product trust boundary:
strict automatic eligibility by default, with explicit per-machine trusted mode.
The shared Ghostty scenario must not be promised as unattended for every template
under strict mode.

Sources: [apply](https://www.chezmoi.io/reference/commands/apply/),
[target types](https://www.chezmoi.io/reference/target-types/),
[hooks](https://www.chezmoi.io/reference/configuration-file/hooks/),
[output](https://www.chezmoi.io/reference/templates/functions/output/).

An empty approved target set must mean no operation in dot: calling chezmoi apply
without targets applies everything. Native update defaults to Git pull with
autostash/rebase, so it does not directly implement the accepted pause-on-local-edit
policy. Use deliberate Git synchronization plus chezmoi operations, with verified
flags. Partial application is possible; do not promise atomic rollback.
Sources: [update](https://www.chezmoi.io/reference/commands/update/),
[global flags](https://www.chezmoi.io/reference/command-line-flags/global/).

### Round 4: accepted file actions, trust and Git behavior

15. **Accepted: file actions.** Separate capture HOME → source, apply source → HOME, restore
    HOME from rendered source, discard source changes against an explicit Git
    baseline, and stop tracking while retaining HOME. Stage/commit/push remain
    distinct. Multiple selection applies to the current action with an exact scope
    summary; switching action must not silently reuse an incompatible selection.
16. **Accepted: automatic template trust.** Strict automatic eligibility is the
    default: unproven dynamic behavior pauses. Offer separate explicit per-machine
    Trusted repository mode, allowing unattended native chezmoi behavior with
    potentially broader effects from repository code. That trust must not be
    silently enabled by a shared repository change. Preserve ordinary interactive
    chezmoi compatibility in either mode. Shared templated settings run unattended
    only if eligible under strict mode or explicitly trusted on that machine.
17. **Accepted: partial failure and recovery.** Stop on failure, show successful/failed/not-run
    targets and changed state, refresh before retry; retain recoverable file backups
    and offer explicit restore. Do not claim arbitrary scripts can be undone.
18. **Accepted: Git scope.** Show the actual Git index and all staged changes; commit review
    includes anything already staged externally. Start with file-level staging,
    deferring hunk staging. Pull fast-forwards only when clean; divergence pauses
    with a clear handoff rather than silently stashing, rebasing or overwriting.

### Facts informing machine reporting and migration

Explicit Git fetch/push refspecs and independent commit construction allow report
branches without changing the configuration branch tip, checkout or index. Report
objects and refs still modify repository storage. Branches are not privacy
boundaries, may trigger existing remote automation, and require write permission.
Reporting can fail independently of applying configuration. Use stable installation
identity plus a display name; never assume the display name uniquely identifies
a machine. Cached reports are dated observations, not proof of online status.
Sources: [fetch](https://git-scm.com/docs/git-fetch),
[push](https://git-scm.com/docs/git-push),
[commit-tree](https://git-scm.com/docs/git-commit-tree).

Migration can preserve original source/config/state verbatim and convert supported
entries, but must not claim universal semantic equivalence. Current templates have
dot-specific data; logical mappings use common → machine → OS precedence; partial
Git configuration management must not silently become whole-file management.
Other machines' local enrollment cannot be reconstructed from the shared catalog.
Evidence: `internal/native/types.go`, `logical.go`, `content.go`, `gitconfig.go`,
and [chezmoi design](https://www.chezmoi.io/user-guide/frequently-asked-questions/design/).

### Round 5: accepted reports, agent integration, migration and navigation

19. **Accepted: machine reports and scheduling.** Opt-in summary reports on a separate branch
    per installation in the same Git remote. Publish revision, timestamp, outcome
    and counts, not configuration values or raw logs. Explicitly disclose that
    reporting automatically commits/pushes reports only; configuration publication
    stays manual. Show local apply, report upload and report retrieval independently.
    Default enabled sync checks every five minutes while awake, with offline backoff
    and no catch-up burst. Reports remain timestamped last-known observations.
20. **Accepted: agent integration defaults.** A local action journal records available actor,
    intent, affected paths and results, with unknown attribution left unknown; no
    raw config values, credentials or command output by default. Keep 30 days of
    local history by default with configurable retention. Init installs a small
    versioned `~/.agents/skills/dot/SKILL.md`, preserving user edits and conflicting
    existing files. Journal shows running/succeeded/failed/partial outcomes rather
    than only successful operations. Remote detailed activity sharing is off by
    default; Q19 summary reporting remains separate.
21. **Accepted: migration.** Offer an explicit previewed conversion, archive original
    source/config/local state, leave live HOME unchanged until a separate Apply,
    and hold ambiguous conversions for resolution. Do not silently migrate on
    startup or rewrite other machines' local enrollment. Preserve profiles and
    custom metadata in the archive until their mapping is explicitly resolved.
22. **Accepted: interaction defaults.** Collapsed HOME hierarchy with All/Managed/Changed
    filters; All shows untracked entries greyed out. Numbers focus panels, Tab
    cycles focus, arrows enter/leave directories, Space selects files or stages in
    Git context, Enter opens details, `/` filters, Esc backs out, `?` shows help.
    Text inputs own typing before global shortcuts. Click/scroll and clickable
    action hints share the keyboard action definitions. On narrow terminals,
    collapse inactive panels and keep all actions accessible through a panel
    switcher/help rather than wrapping or dropping controls. Final exact keymap
    and layout breakpoints belong in the reviewed spec/prototype.

### Consolidated review and final policy frontier

The [workflow walkthroughs](tui-redesign-walkthroughs.md) consolidate Q1–Q22 and
trace all confirmed bug IDs. They are a design review artifact, not a claim of
implementation or a finalized spec. Three remaining product decisions are explicit:

23. **Accepted: local enablement versus shared tracking.** Expose Enable/Disable
    automatic updates on this machine separately from Apply/Restore and Stop
    tracking. Local changes are incremental and retain live files; selected-file
    Apply/Restore never replaces unrelated scope. Stop tracking edits shared source
    and shows that publication affects other machines. Ordinary chezmoi repositories
    can be inspected and operated manually without enrolling in dot automation.
24. **Accepted: concurrent changes and selection freshness.** Dot's TUI, CLI and
    background worker serialize writes to the same source/destination. Refresh
    after external changes; invalidate a review if its files, rules or Git baseline
    change. Do not reuse stale approval or silently retry a changed destructive
    action. External tools need not honor dot's lock; detect what is observable
    and disclose limits. No claim that arbitrary template evaluation is sealed.
25. **Accepted: utility access and existing features.** Keep Settings, Packages,
    recovery and source switching accessible from a global action menu with
    contextual help, rather than additional permanent panels. Preserve existing
    supported utility capabilities; no new package engine. Complete source
    replacement through preview/archive/result and report backups only after
    creation, with failure recovery. Exact UI forms and command names go in spec.

After this frontier is answered, confirm the consolidated walkthroughs as the
shared understanding and generate the spec with traceable acceptance cases.

### Spec coverage checklist (decisions resolved above; details for the spec)

- File identity and layers → tracked/untracked/changed filters, logical variants,
  rendered templates, manifest editor, current-machine versus other-machine views.
- Git scope → staging/hunks, commit selection, unrelated index work, branch and
  stash behavior, pull divergence/conflicts, fetch freshness, failure recovery.
- Action cadence → concrete save/restore/discard/enroll/untrack scenarios, backups,
  plan freshness, multi-file partial failure and interrupted-operation recovery.
- Conventions → focus, pane numbering, shortcuts, mouse/touch parity, selection
  persistence, search ownership, expanded view, narrow terminals and help.
- Configuration scope → template/rendering behavior, variant editing, compatibility,
  and any explicit policy or architecture revisions.
- Agent integration → intent/execution/result records, provenance, external edits,
  concurrent user work, live observation and retention/privacy rules.
- Cross-machine scope → identity/catalog, intended versus observed state, freshness,
  offline machines, report transport and any remote connection/authorization model.
- All above → annotated layouts, end-to-end walkthroughs, acceptance matrix,
  migration/compatibility boundaries, implementation sequence and final spec.

## Documentation rules for this interview

- Record answers immediately, distinguishing accepted, rejected and open choices.
- Add settled domain terms to root `GLOSSARY.md`; keep implementation out of it.
- Create an ADR only for a settled consequential trade-off that would otherwise
  surprise a future maintainer. Existing ADRs 0001–0004 remain in force until
  explicitly revised; ordinary layout choices belong here or in the spec.
- Keep each bug's identity in the register; link spec acceptance cases to it.
- The grilling skill requires shared-understanding confirmation before executing
  the resulting redesign. Documentation and factual investigation proceed now.

## Answers

Round 1, user: “accept 1-4 recommendations.” Decisions recorded above.
Before deciding question 5, the user requested a factual comparison of which
chezmoi features dot currently reimplements.
The [capability comparison](chezmoi-capability-comparison.md) confirms substantial
overlap and revises question 5 into an engine-ownership decision: retain the native
file engine, or delegate overlapping mechanics to chezmoi with dot owning policy
and UX.

Round 1 follow-up, user: “5. A. I want chezmoi to do the heavy lifting not us.
dot just provides a tui for users to easily work with chezmoi and also to see how
agent is managing their dotfiles between different machines.” Accepted in ADR
0006. Migration, supported scope and recovery must be proved before finalizing the
spec; no remote execution or automatic actor attribution has been promised.

Round 2, user accepted Q7 C and requested an init-installed `.agents/` skill.
Instead of choosing Q8 A/B, user asked about common plus per-machine configuration
and automatic propagation of shared values. That scenario is recorded above.

Round 2 follow-up, user: “yes to all.” Accepted the preceding recommendations for
Q6 (ordinary chezmoi compatibility), Q9 (declared edit scope), and Q10 B (opt-in
automatic pull and apply). Q8 remote execution was not part of that acceptance.

Round 3, user: “all looks good.” Accepted Q11–Q14: persistent workspace layout,
independent background synchronization (also resolves Q8), shared → OS → machine
inheritance, and review boundaries for automatic application. This accepts the
product policy, not an unproved guarantee for arbitrary executable chezmoi sources.

Round 4, user: “all recommended.” Accepted Q15–Q18: explicit directional file
actions, strict automatic mode with optional local trust, honest partial-failure
recovery, actual-index commit review, file-level staging, and clean fast-forward
pulls with conflict/divergence handoff.

Round 5, user: “all recommended.” Accepted Q19–Q22: opt-in per-machine Git report
branches, five-minute local sync checks with offline backoff, local 30-day activity
history and init skill preservation, explicit preview/archive migration, and
consistent hierarchical keyboard/mouse/responsive navigation.

Round 6, user: “all recommended” for Q23–Q25, then explicitly requested deleting
the Go implementation and rewriting the app in Rust + Ratatui. This confirms the
shared workflow understanding and authorizes implementation. ADR 0010 records the
new implementation choice; previous engine/UI choices do not constrain the rewrite.
