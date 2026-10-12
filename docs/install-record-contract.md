# Local install and connection state contract

This is the shared #34/#35 interface. By default, state is local-unmanaged under
`${XDG_STATE_HOME:-$HOME/.local/state}/dot/`; it is never stored in either Git
checkout, a release archive, chezmoi data, approvals, or recovery journals.

An installer `--state-dir ABSOLUTE_PATH` selects the exact runtime state
directory, not merely the install record location. Its generated launcher pins
that path as `DOT_STATE_DIR` on every invocation, overriding inherited XDG or
DOT_STATE_DIR values. The path is shell-quoted, must be normalized and absolute,
and may not traverse symlinks. The CLI, Python engine, read-only `dot init
--check`, self-update ownership checks, approvals, intent journals, migration,
and recovery all resolve through this same value. A bad explicit value fails
closed; it never falls back to the legacy default. Direct CLI users without an
installer-pinned launcher may set `DOT_STATE_DIR` explicitly; absent that, the
historic XDG/default behavior is unchanged. No state is moved, copied, or
deleted when selecting a different directory.

## Public artifact record (installer-owned)

`install.json` is schema `record_version: 2` for current artifact installs. It describes the installed public
tool artifact only; a private checkout or tracker is never required to install
or identify it:

```json
{
  "record_version": 2,
  "owner": {"kind": "artifact", "manager": "curl"},
  "artifact": {
    "repository": "ujjwal6792/dots-cli",
    "release_tag": "v1.2.3",
    "platform": "linux-amd64",
    "sha256": "<64 lowercase hex>"
  },
  "install_root": "/home/example/.local/share/dot-cli",
  "engine_path": "/home/example/.local/share/dot-cli/current/tools/dot",
  "state_dir": "/home/example/.local/state/dot-cli",
  "browser_sha256": "<64 lowercase hex>"
}
```

The owner enum is `artifact/curl` or `package-manager/brew|pacman`. Artifact
install/update may replace this record only when it is a valid schema-v1
artifact/curl record or a valid schema-v2 record whose `state_dir` equals the
directory containing it. Version 1 remains supported for existing installs;
new artifact installs write version 2. Package-manager ownership is read-only to the installer and
`dot self-update`; direct users to the manager. Unknown, malformed, legacy, or
symlink-redirected records fail closed and are never silently upgraded or
reinterpreted. The archive checksum is the reviewed release integrity input;
this mutable record is diagnostic ownership data, not a trust root.

## Private connection record (init-owned)

`connection.json` is an independent schema `connection_version: 1` record:

```json
{
  "connection_version": 1,
  "private_source": "/home/example/.local/share/dot-config",
  "config_path": "/home/example/.config/chezmoi/chezmoi.toml"
}
```

Both paths are absolute. Absence means not connected; it is a normal pre-init
state, not corruption. Only `dot init` may create/update this record, after its
existing read-only detection and explicit private connection/tracker consent.
Connection changes must not rewrite, replace, migrate, or change modes of
`install.json`, even when the public artifact is installer-owned. Read operations
must not create either record. Writes use a same-directory temporary file,
mode 0600, and atomic rename; parent state directory mode is 0700. Do not touch
approvals, other local state, or recovery data as part of either operation.

## Shared Python API

`tools/dot/install_state.py` is the sole serialization/writer seam for the CLI:

- `read_install_record(state_dir)` validates and returns the public record, or
  `None` when absent; it never creates state.
- `read_connection(state_dir)` validates and returns the private connection, or
  `None` when absent; it never creates state.
- `write_connection(state_dir, private_source, config_path)` atomically updates
  only `connection.json`.

The installer is shell-only and writes the documented install schema itself with
same-directory temp + rename; it does not call Python or the connection API.
`dot self-update` checks owner first and uses `connection.json` only when present
to validate candidate compatibility. A missing connection never blocks public
artifact install/update. API validation rejects unknown keys, wrong types,
non-absolute paths, malformed artifact hashes, and unsupported schema versions.
