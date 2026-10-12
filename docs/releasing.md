# Public release procedure

Releases are owner-initiated GitHub prereleases, built only from the exact
current tip of the public repository's merged `main`. Do not hand a local
candidate or a short-lived Actions artifact to an installer.

## Before dispatch

1. Confirm the public release PR is merged, the intended public `main` commit
   is reviewed, and the private cleanup required by the project plan is
   complete. Confirm there is no in-progress `main` update.
2. Review the successful public test and security results. The release workflow
   repeats the public test suite, scans the complete fetched public Git history
   with gitleaks, builds all three supported assets, checks package contents
   and checksums, and runs installed-package fixture smoke tests on a clean
   GitHub-hosted Apple Silicon runner.
3. Select and approve the exact version. The workflow accepts only a new
   `v0.1.0-rc.N` with `N >= 2` and greater than every existing RC. Existing
   `v0.1.0-rc.1` is immutable. `v0.1.0-rc.2` is not preselected by this
   procedure; the owner must choose the input at dispatch time.

## Dispatch and review

In `ujjwal6792/dots-cli`, open **Actions → Publish reviewed release → Run
workflow**, select the `main` branch, enter the owner-approved exact value in
`approved_version` (including the leading `v`), and confirm. Equivalent CLI
invocation after authorization is:

```sh
gh workflow run release.yml --repo ujjwal6792/dots-cli --ref main \
  -f approved_version="$OWNER_SELECTED_VERSION"
```

Set `OWNER_SELECTED_VERSION` to the exact version selected after review; the
command above does not select a version by itself.

The dispatch commit must still equal remote `main` at the beginning of every
stage and immediately before publication. The gate rejects a branch, fork,
non-dispatch event, stale main commit, malformed version, existing tag, or
non-monotonic RC number. A workflow failure at any stage prevents publication.
Do not manually create the version tag or release to bypass the gate.

The publish job serializes this workflow by approved version and stages assets
on an unpublished draft. A same-version retry can reuse a draft only if its
identity and complete asset names, sizes, and SHA-256 digests match the fresh,
reproducibly built candidate. If the draft targets an older commit, its source
must additionally be proven by a prior successful
verify/build/installed-smoke run, be an ancestor of current main, and have no
existing version tag. A stale draft is retargeted by release ID while
remaining a draft, fetched again, and fully rechecked before publication. After
publishing, the workflow verifies the release by ID and checks that the Git
tag resolves to the approved main commit. Any mismatch fails closed without
deleting or recreating a draft or tag.

Concurrency caveat: the workflow concurrency group prevents two runs of this
workflow for the same version from publishing simultaneously. It does not lock
the release against repository owners or collaborators editing it through the
GitHub UI or another API client. GitHub's documented Update Release API does
not specify a conditional-write/compare-and-swap mechanism, so an external
authorized edit can still race between the final read/asset verification and
the publish PATCH. The post-publish readback detects a mismatch but cannot
undo publication. Before dispatch, the maintainer responsible for the release
must ensure no authorized person or other automation will edit that draft
during publication and explicitly decide whether this residual risk is
acceptable. If it is not accepted, do not dispatch; preserve the draft and
choose a separately reviewed recovery plan.

If post-publish verification fails, do not direct users to the release; inspect
the workflow and published state and escalate rather than automatically
deleting or replacing assets/tags. Before installer handoff, require a
successful final workflow readback for the exact source commit and tag, review
all three archives, all eight expected asset names/sizes/digests, release
notes, and the included `SHA256SUMS` and `BROWSER_SHA256SUMS`. Verify checksums
on downloaded files before installation. The assets are for macOS Apple
Silicon, Linux x86_64, and Linux ARM64. There is no native Linux install test
or package-manager installation behavior in this workflow. Checksums detect
corruption and cross-check the browser in each archive; they are **not** a
cryptographic build attestation or independent provenance proof. The
GitHub-hosted installed-package fixture smoke is bounded test evidence, not
validation on a target owner's machine. No credentials or private-repository
authentication are bundled; users need their own GitHub CLI authentication
and access to their private configuration repository.

Never add private manifests, inventory, credentials, backups, or private
history to the public source or release assets. Do not trigger a release as
part of development or issue validation; publication requires the later
owner-selected dispatch after the project sequencing gates are met.
