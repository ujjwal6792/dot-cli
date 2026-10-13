# Machine assignments and local adoption

This is the retained #15 behavioral contract. The Python implementation was
retired; #16 must implement and verify it in the native Go engine. Current UI
tests alone do not establish persistence or policy enforcement.

New Track entries default to:

```json
{"machines": {"include": ["all"], "exclude": []}}
```

`all` is the only wildcard, in either list. All other values are exact machine
identities, not patterns. Exclusions win. Include must be nonempty; duplicate
ids, unknown assignment fields, invalid ids and non-list values fail at intent
plan. Missing assignment metadata in an older entry means all machines; reading
an older manifest never rewrites it. Existing logical-file machine lists remain
readable, while newly tracked logical files use the include/exclude assignment.
Old engines reject the new metadata rather than dropping its restrictions.
Contract/approval negotiation remains protocol 1.

## Terminal controls

In the Dotfiles Track browser, **Tab** retargets the highlighted exact file:
all → available machine identities → all. Identities come from the private
machine catalog, assignment references, and this target's adoption v3 identity.
The assignment is shown alongside the file. This only prepares intent: **p**
generates and freshness-checks the exact plan, followed by the existing separate
Unsafe/desktop/backup gates and cancel-default final confirmation. Track changes
private source, never adopts the target. Logical mappings and bounded Git entries
retain their existing capabilities and boundaries.

**Shift+Tab** changes Track/Adopt direction. In Adopt, **Space** selects or
deselects an exact file locally. Tab does not edit shared assignments from Adopt;
switch to Track for a shared assignment change. Even an explicitly empty local
selection must be reviewed and approved; entering Adopt never authorizes files.

In Adopt, **s** opens named manifest profiles. Selecting another profile adds its
flat list to the basket as a union (duplicates are selected once), not a replacement
bucket. Space can deselect members afterwards. The basket freezes exact ids; no
future profile additions gain automatic consent.

## Scriptable intent and ownership

After setting up this target's machine identity, compose profiles with a request:

```json
{
  "version": 1,
  "mode": "adopt",
  "profile": "my_selection",
  "profiles": ["editor", "shell"],
  "entries": ["editor_preferences"],
  "metadata": {"editor_preferences": {"tools": ["local_editor"]}}
}
```

`profile` labels the local approval; `profiles` lists existing named manifest
profiles. Without `entries`, the planner resolves their union and filters OS,
shared assignment, and persisted local deselections. With `entries`, those exact
ids are the explicit selection; an inapplicable id is rejected rather than silently
adopted. `metadata` can customize only selected entries and cannot edit assignments,
source/destination mappings, or unknown capabilities. Invalid requests write nothing.

Adopt never rewrites shared entries or named profiles. Adoption v3's machine-bound
record stores optional `profiles`, `deselected`, and `customs` fields locally. Customs
survive deselection/reselection and apply only to that target's resolved view.
Updates use only the existing exact approved scope, retain the local customizations,
and do not add deselected files. Shared manifest changes still require a separate
reviewed source-side plan. Changing local state or source invalidates a saved plan.
Machine-2 onboarding and status/diff JSON are separate work (#16).
