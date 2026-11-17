# Import current files from an existing setup, never its history

`dw import --bare DIR` (and `--from-file LIST` or stdin) reads the file list of an existing dotfiles setup and turns it into ordinary track pending items for the files as they are in HOME now. This reverses the v2 non-goal "no importing existing setups": the most common way people arrive is from a bare git repository in HOME, and re-tracking hundreds of files by hand was the step that stopped the owner from backing up at all. Import is a front end to `track`, not a converter: it adds no repository content of its own, so assignments, hide rules, secret detection and the review of pending items apply unchanged. History is not carried over (ADR 0001); the first commit names the old repository and its last commit for reference.

## Considered options

- Documented `git ls-files | dw track` recipe only: rejected because one hidden or secret path among hundreds fails the whole batch, and the recipe cannot group files into tracked folders.
- Grafting the old history into the chezmoi layout: rejected as fragile (every path is renamed to source form) and of little value next to a fresh start.
- Reading chezmoi source or stow packages directly: deferred; `--from-file` covers them through a path list.
