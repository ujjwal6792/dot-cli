---
name: setup-dots
description: Guide setup of the Go dot CLI, user-owned private source, machine identity, dependencies, and tracker with explicit consent.
---

# Set up dot

## Development status

The interpreter-based CLI and its installers were retired. This checkout has
Go UI components but no functioning setup engine yet. Issues #17 and #18 own
native setup and delivery. Consult installed `dot --help` before promising a
command; stop if it reports that the native runtime is unfinished.

## Target setup flow

1. Use the reviewed native `dot` executable. Git and chezmoi are its runtime
   dependencies. Release packaging and installation instructions are pending #18.
2. Run `dot init --check` for offline, read-only detection when implemented.
   Confirm the user's private source, target HOME, tracker config and state paths.
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

For runtime-independent acceptance requirements, see
[`docs/native-rewrite.md`](../../docs/native-rewrite.md). Implement missing
behavior in the shared Go engine, not in agent-created shell/Python workarounds.
