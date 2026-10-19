# Standard chezmoi repositories work without a dot manifest

Dot will support ordinary chezmoi repositories without requiring conversion to a
dot-specific repository contract. Chezmoi source state remains authoritative for
file behavior. Optional dot metadata can enrich configuration relationships,
machine visibility and recorded activity without becoming a second file engine.

A mandatory manifest would simplify dot-specific assumptions but make the TUI
dependent on a proprietary catalog and duplicate chezmoi's model. The user chose
ordinary repository compatibility in the 2026-10-19 design interview, consistent
with delegating file management to chezmoi in ADR 0006.

Enhanced views must distinguish unavailable metadata from empty or healthy state.
Arbitrary existing templates need not expose editable setting-level provenance.
Migration of the existing manifest and enrollment remains a separate design
decision. This ADR records the accepted redesign direction; it does not claim the
current implementation already supports it.
