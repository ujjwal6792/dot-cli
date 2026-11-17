# Bundled chezmoi, git and delta with an isolated store

Dotwarden ships pinned chezmoi, portable git (dugite-native) and delta, and keeps its repository, chezmoi config and chezmoi state in its own XDG directories, passing explicit paths on every call. A user's own chezmoi setup is never read or modified, and upstream tool changes cannot alter behaviour. This reverses v1's "system git" choice. The cost is shipping and updating git (GPLv2 licence obligations, glibc compatibility on Arch/CachyOS). The user's `~/.gitconfig` is still read for identity and signing, and system `ssh` with the user's keys handles SSH remotes, so authentication keeps working.

## Considered options

- System git: rejected so behaviour and versions are identical on every machine.
- gitoxide in-process: rejected because its push and merge support is incomplete.

## Consequences

Authentication: SSH remotes use the system `ssh` and agent; HTTPS remotes use the credential helpers (`credential.helper`, `credential.<url>.helper` and related keys) from the user's global git config, passed through in order. The public-remote probe never receives them. Hooks, URL rewrites and aliases stay blocked.
