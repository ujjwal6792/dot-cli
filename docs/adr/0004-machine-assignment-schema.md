# Assignment eligibility and local approval are separate

Entries add `machines: {include: ["all"], exclude: []}` under protocol 1; include
must be nonempty, only `all` is a wildcard and exclusions win. Missing older entry
metadata means all, and legacy lists are accepted only in old logical-file metadata,
not new entries/requests. Shared eligibility changes need exact Track approval;
adoption-v3 deselections/customs remain local, survive profile switches, and cannot
weaken shared Unsafe authority. Profile unions are frozen into exact ids, so future
source additions cannot enter an existing Update approval.
