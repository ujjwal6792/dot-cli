---
name: setup-dots
description: Public-safe setup guidance for the private dot system — download/review/execute, consent, access verification, and the Track/Adopt split. Bundled with the public bootstrap; contains no private configuration.
---

# setup-dots

Guide a user through setting up the private `dot` system from the public
bootstrap. This skill is public-safe: it contains no private configuration,
credentials, history, or target inventory.

## 1. Obtain and review the bootstrap

- Present the download/review/execute route: fetch `bootstrap.sh` over HTTPS
  with `--proto '=https' --tlsv1.2`, verify its SHA-256, read it, then run it.
- The piped `curl | bash` route is documented, but remind the user that
  interactive prompts and browser login still use `/dev/tty`.
- Never claim a working public URL before the release owner provides one.

## 2. Dependency plan and consent

- Explain the platform route (macOS Homebrew bottles; Ubuntu apt + pinned
  upstream packages; Arch official pacman with a disclosed full upgrade).
- Show the exact package/repository plan before any consent. Confirm the user
  understands sudo, third-party repositories, and Arch full-upgrade impact.
- `--yes` still requires existing identity, verified access, and `sudo -n`;
  it never selects configuration groups.

## 3. Git identity is preserved

- Preserve configured commit name/email; prompt only for missing values.
- Do not generate SSH keys, reset signing, or rewrite unrelated Git settings.

## 4. Authentication and access verification

- Verify access to the private dotfiles repository (API and Git) before any
  private retrieval. Reuse existing GitHub credentials only after that check.
- On failure, offer account switching or HTTPS browser login guidance, then
  recheck once. Do not print tokens or retry forever.

## 5. Private source: use or explicitly approved creation

- Prefer cloning/using a verified existing private dotfiles repository.
- Creation of a NEW private repository requires its own explicit consent:
  show the proposed owner/name and private visibility, and require typed
  approval. A refused consent means **no remote change**.
- Never use creation as a fallback for denied access, never delete/rename
  another repository, and never adopt a denied or unrelated repository.
  `scripts/create-private-repo.sh` implements this consent-first flow.

## 6. Distinct Track and Adopt rounds

- Track (import eligible HOME configuration) and Adopt (apply repository
  configuration) are separate picker rounds with distinct previews.
- Browsing is not consent; Track does not authorize Adopt. Adopt applies only
  the explicitly approved exact scope, with Unsafe selections requiring an
  additional acknowledgment.
