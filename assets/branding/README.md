# Welcome screen marks

`dot.svg` is the original dot mark: an enclosing pair of file brackets with a
single dot at its center. It uses the workspace's mint accent.

`chezmoi.svg` is the official white mark from https://www.chezmoi.io/logo.svg.
Copyright Tom Payne; the upstream MIT license is included in `chezmoi-LICENSE`.

`git.svg` is the official orange mark from
https://git-scm.com/images/logos/downloads/Git-Icon-1788C.svg.
Git Logo by Jason Long is licensed under
[Creative Commons Attribution 3.0 Unported](https://creativecommons.org/licenses/by/3.0/).
See https://git-scm.com/community/logos.

The TUI uses a half-block adaptation of `dot.svg` in `src/onboarding.rs`.
The chezmoi and Git SVGs remain brand references. Their large terminal
adaptations have been replaced by a single-line Nerd Fonts powered-by row:
`nf-md-home` (U+F02DC) for chezmoi and `nf-dev-git` (U+E702) for Git.
The [Nerd Fonts glyph catalog](https://github.com/ryanoasis/nerd-fonts/blob/master/glyphnames.json)
has no dedicated chezmoi glyph. Names accompany both icons.

These two glyphs require a Nerd Font in the terminal. `DOT_ICONS=none` selects
text-only labels; very narrow terminals also use text-only labels. Font coverage
is not auto-detected. No terminal image protocol is required.
