# Dot configuration

Vocabulary for deliberately sharing dotfiles while keeping adoption local.

## Language

**Logical configuration file**: One configuration as the user understands it,
with its machine-specific destinations and operating-system variants treated as
parts of the same file.
_Avoid_: treating each source variant as an unrelated configuration

**Shared setting**: A configuration value intended for multiple machines, inherited
by each machine that does not explicitly override it.

**OS override**: A configuration value specific to an operating system that takes
precedence over the corresponding shared setting.

**Machine override**: A machine-specific configuration value that takes precedence
over corresponding shared and OS-specific values for that machine.

**Machine**: A named target environment with its own identity and adoption choices.
_Avoid_: personal/work profile, group

**Machine report**: A timestamped summary of a machine's observed configuration
revision and operation outcome. It describes the last reported state, not current
availability.

**Activity record**: An observation of an operation's intent, affected
configurations and outcome, with actor information when available.
_Avoid_: attributing an unexplained external edit to a known actor

**Machine-assignment**: Shared eligibility for an entry, expressed as included and
excluded machine identities. Eligibility is not permission to apply.
_Avoid_: approval, adoption scope

**Track**: A reviewed import of exact target files or metadata into shared source.

**Capture local changes**: Update managed source from changes made to a local
configuration, resolving the intended scope when it is ambiguous.

**Apply**: Update local destinations to match the desired configuration rendered
from managed source.

**Stop tracking**: Remove a configuration from managed source while retaining its
local destination.

**Adopt**: A reviewed enrollment of exact source entries on one machine.

**Profile**: A named flat list of source entries that can be composed with other lists.
_Avoid_: machine identity, personal/work tag
