---
name: setup-dots
description: Guide setup of the Go dot CLI, user-owned private source, machine identity, dependencies, and tracker with explicit consent.
---

# Set up dot

## Setup flow

1. Use the reviewed native `dot` executable. Git and chezmoi are its runtime
   dependencies. Release bundles privately pin chezmoi v2.73.1 and use system
   Git. Verify a trusted archive checksum before owned-root installation;
   [`docs/native-distribution.md`](../../docs/native-distribution.md) owns the commands.
2. Run `dot init --check` for offline, read-only detection.
   Confirm the user's private source, target HOME, tracker config and state paths.
   An old-user verdict checks source/contract, not overall setup completion.
3. Open `dot init`. Connect a user-supplied HTTPS/SSH URL using system Git.
   The user creates an empty remote repository on their hosting platform if
   needed; the CLI only initializes/clones locally and configures origin.
4. Preserve existing source/Git data. Replacement requires an exact named backup
   and a second confirmation; denied access is never a reason to replace data.
5. Review editable machine identity, dependency operations and tracker creation.
   Each write needs the relevant explicit consent; existing machine data is
   preserved. Native package managers own their authentication prompts.
6. Offer separate Track and Adopt rounds with nothing preselected and Finish
   as default. Installation and browsing never authorize configuration changes.

For scripted source/identity/tracker operations and path precedence, read
[`docs/native-setup.md`](../../docs/native-setup.md). URL consent only authorizes
source creation, not identity, tracker or adoption. For machine-2 import policy,
read [`docs/machine-assignments.md`](../../docs/machine-assignments.md).
Report unsupported operations explicitly; private executable templates are not
run. For verification/platform limits, use
[`docs/native-acceptance.md`](../../docs/native-acceptance.md).
