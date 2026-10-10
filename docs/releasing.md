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
non-dispatch event, stale main commit, malformed version, existing tag/release,
or non-monotonic RC number. A workflow failure at any stage prevents release
creation; assets are published together only after all checks pass. Do not
manually create the version tag or release to bypass the gate.

The publish job stages assets on an unpublished draft, compares GitHub's
reported SHA-256 digest and exact asset-name set with the complete local
package, and only then makes the prerelease public. If that final job fails,
inspect the workflow and any unpublished draft/tag before taking action; the
same version is intentionally blocked from automated reuse to prevent
clobbering an existing tag or release.

Before directing users to an asset, review the completed workflow and GitHub
Release page: source commit, all three archives, `SHA256SUMS`,
`BROWSER_SHA256SUMS`, and the prerelease notes. The assets are for macOS Apple
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
