---
name: setup-dots
description: Public-safe setup guidance for the private dot system — download/review/execute, consent, access verification, and the Track/Adopt split. Bundled with the public bootstrap; contains no private configuration.
---

# setup-dots

Guide a user through setting up the private `dot` system from public `ujjwal6792/dots-cli`
bootstrap. This skill is public-safe: it contains no private configuration,
credentials, history, or target inventory.

## 1. Obtain and review the bootstrap

- Present the download/review/execute route: fetch `bootstrap.sh` over HTTPS
  with `--proto '=https' --proto-redir '=https' --tlsv1.2`, verify its SHA-256,
  read it, then run it.
- The piped `curl | bash` route is documented, but remind the user that `dot
  init` itself is non-interactive (it stops with guidance unless given an
  explicit `--url`); only the prebuilt browser owns a terminal.
- Never claim a working public URL before the release owner provides one.
- Use independent locations: public CLI `~/.local/share/dot-cli`, private source
  `~/.local/share/dot-config`, launcher `~/.local/bin/dot`. `DOT_SOURCE` remains
  private; `DOT_ENGINE` names public `tools/dot`. Confirm both locations and
  repository identities before installing. Matching installed checkouts are
  detected read-only; updates are separate commands. Legacy checkouts require
  separate reviewed migration, not replacement.
- Use the reviewed platform's prebuilt browser: Go is a maintainer build
  dependency, not a target prerequisite. Python/chezmoi remain required.
  Compare the archive SHA-256 against an independently reviewed digest before
  extraction/execution; a checksum from the same URL is not provenance.

## 2. Init owns dependency and tracker creation

- Bootstrap detects prerequisites read-only and installs independent CLI/source
  checkouts and a launcher after installer consent. It does not install
  dependencies or create/repair a chezmoi tracker.
- `dot init` owns first-time dependency installation and tracker setup or
  detection. Before each install/create, show the exact operation and obtain
  informed consent; refusal leaves that operation undone. Detection does not
  create, repair, switch or widen scope. Start with `dot init --check`; then use
  guided `dot init`, which offers Finish by default. New tracker creation previews
  its exact actual source/HOME/config/engine and preserved Git identity; private
  templates/hooks are not executed. Existing machine data stays intact.
- Explain proposed init platform effects: Homebrew bottles, Ubuntu named apt/
  reviewed upstream packages, or Arch's disclosed full system upgrade.
  Dotfiles browsing, self-update and migration have no package-manager effects.
  Init's `packages` action and browser Packages Install missing each need their
  own exact plan-ID approval; edits/navigation/deselection never install/uninstall.
- `--yes` is installer consent only, never dependency/tracker/adoption consent.

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
  configuration) are separate browser rounds with distinct previews. Finish is
  the default; no files are preselected. Installing CLI alone leaves an empty
  adoption scope and changes no target configuration. Zero managed drift does
  not mean whole-machine parity, especially when selected-entry count is zero.
- Browsing is not consent; Track does not authorize Adopt. Adopt applies only
  the explicitly approved exact scope, with Unsafe selections requiring an
  additional acknowledgment.

## 7. Maintenance and compatibility

- Resolve public engine and private source independently. Missing/incompatible
  contracts stop before mutation; repair those paths/versions rather than
  falling back to unscoped chezmoi apply or private executable tooling.
- Keep tooling updates (`dot self-update`), approved private updates (`dot pull`),
  and application version-manifest setup (`dot update NAME VERSION`) distinct.
  Browsing does not provision applications or services. Consult installed help
  for commands delivered by the reviewed version; do not promise pending commands.
- For machine-data diagnostics use the installed private `home/` and actual
  `DOT_CONFIG`. Preserve identity and machine data; deletion is not routine repair.
