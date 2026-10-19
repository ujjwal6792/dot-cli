> Historical Go baseline. The Rust rewrite supersedes this implementation; see [Rust spec](rust-rewrite-spec.md) and [current verification](rust-acceptance.md).

# Local machine identity

This is the native setup identity contract. The real extracted app's edited
identity journey and machine isolation are recorded in [acceptance](native-acceptance.md).

After connecting or continuing a source, `dot init` opens an editable machine
identity page. Merely opening the page writes nothing. Enter confirms the id;
Escape quits without saving. Identity setup does not adopt files or install a
tracker, and never contacts the network.

Suggestions use `macos` on Darwin, otherwise normalized `/etc/os-release` `ID`,
then `ID_LIKE`, then `linux`, followed by the normalized OS username. New ids
contain only lowercase `a-z`, digits and underscores (1–128 characters). A
collision with a known source-catalog machine adds `_` and two random lowercase
alphanumerics. Edited collisions are rejected, not silently renamed.

The target-local state directory (including `DOT_STATE_DIR`/`XDG_STATE_HOME`
overrides) owns `adoption.json` v3:

```json
{"version":3,"profile":"unapproved","entries":{},"machine":{"id":"macos_ace","os":"darwin"}}
```

`machine.os` remains the engine's canonical applicability OS (`darwin`/`linux`),
not the distribution token used in suggestions. Existing v3 identities are
reused and cannot be replaced through init. A second HOME/state directory can
choose its own unique id without rewriting the first machine's state or the
shared catalog. Unpublished identities in disconnected state directories cannot
be discovered; uniqueness is against the locally available source catalog.

V3 is the identity source of truth, including for v1 manifests; subsequent
Adopt preserves it rather than downgrading to v2. The existing contract protocol
and manifest/approval capability negotiation remain required. Legacy group scopes
are not migrated by identity setup; malformed records and symlinked state paths
fail closed. Assignment and adoption policy are owned by #16.
