# Automatic execution trust belongs to each machine

Automatic synchronization will use strict eligibility by default. Operations
whose effects cannot be established within the approved automatic scope pause for
review. A machine may separately opt into Trusted repository mode, permitting
unattended native chezmoi behavior, including potentially broader effects from
repository code. Shared configuration cannot silently enable that local trust.

The user accepted this trade-off in the 2026-10-19 design interview. Chezmoi
templates, hooks, modify scripts and externals can execute code; an explicit target
list or a dry run does not sandbox that behavior. Universal unattended template
support would therefore conflict with a strict promise of only updating approved
files. Restricting all usage would undermine ordinary chezmoi compatibility.

The distinction controls automatic execution, not whether dot can open a repository
or offer explicit native operations. Strict eligibility needs an evidence-based
implementation; filename or keyword filtering alone is not proof. No automatic
rollback of arbitrary code is promised. Partial results and recoverable file
backups must be visible. This records a future product contract, not an implemented
security boundary.
