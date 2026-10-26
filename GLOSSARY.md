# Dotwarden

Vocabulary for one person keeping their dotfiles, secrets and preferences in sync
across several machines through one self-contained tool.

## Machines and targeting

**Machine**:
A named computer registered in the shared repository, with its own platform and
distribution. Its name is chosen by the user and is independent of its hostname.
_Avoid_: host, profile, personal/work tag

**Machine registry**:
The shared list of known machines and their platform, distribution and
distribution family.
_Avoid_: machine catalog, inventory

**Platform**:
The operating-system kernel family of a machine, such as darwin or linux.
_Avoid_: OS (ambiguous with distribution)

**Distribution**:
The exact operating-system flavour of a machine, such as cachyos or ubuntu.
_Avoid_: distro ID, OS

**Distribution family**:
A group of related distributions, such as arch-like, matching a distribution and its
derivatives.

**Target**:
One platform, distribution, distribution family or machine that an assignment
names.

**Assignment**:
The set of included and excluded targets deciding which machines receive a tracked
file. A file applies when any include matches and no exclude matches; no includes
means every machine.
_Avoid_: scope, eligibility, approval

## Changes

**Tracked file**:
A HOME file whose content is kept in the shared repository.
_Avoid_: managed file, saved copy

**Tracked folder**:
A folder whose existing files are tracked and in which new files appear as
untracked changes to stage, never added automatically.

**Change**:
A difference between a tracked file in HOME and its committed version, shown like a
modified file in Git.
_Avoid_: drift

**Stage**:
Mark a change, or an untracked file, to be included in the next commit.
_Avoid_: capture, track, add

**Discard**:
Return a HOME file to its committed version, abandoning the local change.
_Avoid_: restore, revert

**Untrack**:
Stop keeping a file in the repository. Every machine keeps its existing HOME copy;
it just stops syncing.
_Avoid_: forget, remove, stop tracking

**Hide rule**:
A shared pattern that keeps noise such as caches and histories out of the file
browser and prevents it from being tracked.
_Avoid_: ignore rule, exclusion

**Variant**:
One of several stored versions of the same tracked file, each with its own
assignment, so one HOME path can hold different content on different machines.
_Avoid_: override file, copy

**Conflict**:
A tracked file changed both locally and by an incoming change, resolved hunk by hunk.

**Incoming change**:
A committed change from another machine that has been pulled but not yet written to
HOME.

**Apply**:
Write incoming changes into HOME.

**Pending action**:
A requested but not yet reviewed operation, such as tracking, assigning or
encrypting, waiting in the pending list for one combined review.
_Avoid_: plan, draft

## Secrets

**Values**:
The table of named secrets and variables, each with a default and per-target
overrides; the most specific matching target wins.
_Avoid_: data, settings

**Vault**:
The encrypted part of the values, holding the secrets.
_Avoid_: keychain, store

**Secret**:
One named, encrypted value in the vault. Its name is visible; its value is not.

**Reference**:
A placeholder inside a tracked file that names a secret or variable and is replaced
by its value for the receiving machine when written to HOME.
_Avoid_: secret reference, template tag

**Encrypted file**:
A tracked file stored entirely encrypted, such as a private SSH key.

**Variable**:
One named, unencrypted value, such as a font size, that tracked files reference and
that can differ per target.
_Avoid_: shared setting, custom setting

**Unlock session**:
A limited period on one machine during which secret values can be read after the
passphrase was entered.

**Recovery code**:
A one-time code shown at vault creation that can unlock the vault and set a new
passphrase when the passphrase is lost.

## Recovery

**Snapshot**:
A local copy of HOME files taken before dot overwrites them.
_Avoid_: backup (ambiguous)

**Archive**:
A single encrypted export of the repository and vault that can restore a machine
offline.

**Point-in-time restore**:
Bringing back one file or every file as it was at a chosen past commit.

## People and tools

**Agent**:
An automated assistant operating dot through its CLI. It may see secret names but
never secret values.

**Preference**:
A setting of dotwarden itself, such as theme, keybindings or the unlock timeout,
kept in an ordinary dotfile that can itself be tracked.
_Avoid_: setting, config
