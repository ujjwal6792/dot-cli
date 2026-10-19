# Replace the Go implementation with Rust and Ratatui

The user explicitly requested deleting the Go application and rewriting dot in
Rust + Ratatui after confirming the redesigned workflows on 2026-10-19. The change
replaces the old application, tests and build tooling, preserving Git history,
product decisions and a recoverable archive of the original source.

This supersedes ADR 0001's Go/Bubble Tea implementation choice. ADR 0006's chezmoi
ownership and the accepted UX remain authoritative. Rust is an implementation
choice, not a reason to recreate chezmoi semantics or preserve the old six-tab
workflow. CLI, TUI and automatic operations share the same backend and review paths.

System Git and private pinned chezmoi remain the release dependencies. The rewrite
has its own native integration and terminal acceptance evidence; historical Go
passes and the earlier Linux waiver are not evidence for this implementation.
