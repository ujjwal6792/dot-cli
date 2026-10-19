# Chezmoi owns file-management behavior

Dot will delegate file import, source-state interpretation, template rendering,
comparison and application to chezmoi. Dot owns the unified TUI, daily Git workflow,
and visibility into how people and agents manage configurations across machines.
The user explicitly chose this boundary in the 2026-10-19 design interview after
reviewing the native engine's overlap with chezmoi.

Maintaining an independent file engine duplicates mature functionality and creates
behavioral differences between dot and chezmoi. Integration should use chezmoi's
supported CLI rather than recreating its semantics. Dot-specific metadata and
activity reporting must have a clear purpose and must not become a parallel source
of truth for chezmoi's file behavior.

This is an accepted architectural direction for the redesign, not a description
of the current implementation. It revises the native file-engine ownership behind
ADR 0001 while retaining one application UI and shared integration paths for any
scriptable dot operations. ADR 0002's pinned binary/system Git choice remains.
The existing manifest, local enrollment and recovery contracts require explicit
migration decisions and acceptance coverage; their current schemas are not assumed
to be permanent product requirements. Exact supported features, observation and
attribution, remote-machine visibility, and migration remain open interview branches.
