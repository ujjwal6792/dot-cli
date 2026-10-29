# T8 report (branch `t8`)

## Built

- `vault.rs`: age X25519 identity; `identity.age` (scrypt, passphrase) and `recovery.age` (generated 32-char base32 code); `init/unlock/lock/status/passwd/recover`. Files are written under the mutation lock and staged. Session store: macOS Keychain (Security framework), Linux Secret Service (`secret-tool`, compile-only here), else a 0600 file in a private dir under `$XDG_RUNTIME_DIR` / temp; payload carries an expiry (default 15 min, `[vault] unlock_minutes` in preferences). `DOTWARDEN_SESSION_BACKEND=file` forces the file store.
- `secrets.rs`: one `.age` per secret (default + overrides, JSON), metadata (name, override targets, placeholder) in the values data file. `values add-secret`, placeholders for agents.
- `render.rs`: hidden `__secret`/`__file` helpers called by chezmoi `output`; per-run token registered as a file named by its SHA-256 (holds the parent pid); helper refuses without a live token and without a session. `dotwarden/value` branches on `kind = "secret"`; `dotwarden/file` renders encrypted files. Masked render (`DOTWARDEN_MASK`) for preview-as.
- `encfile.rs`: `dw encrypt PATH` (pending `encrypt` item; TUI `E`); staging an edited file re-encrypts.
- `detect.rs`: scanner (key patterns, secret-looking assignments, entropy tokens), `secret_move` pending items (no value inside), exact-match wiring of existing secrets (unlocked), commit guard, `redact` on all diffs, `git prune` after a value leaves plaintext.
- Masking: diffs use `«secret:NAME»`; edited positions hidden; status/diff say "Secret NAME changed"; `Row`/JSON carry `••••••`. TUI shows values only while unlocked (`Reveal` is not `Serialize`).
- TUI: Values screen secrets, `u` unlock (masked), `L` lock, `s` new secret, `E` encrypt; Values screen now loads in a job (T7 gap).

## Decisions (please check)

1. **Locked = not classified.** Files needing the vault are skipped by scan/apply/status while locked (listed as "Vault locked: N file(s)"); naming one exits 4. Staging a secret-bearing file needs an unlock.
2. **Detection runs after track/stage** (as T7), so plaintext can sit in the index until the pending move is applied. Mitigations: commit refused on key-like text; `git reflog expire` + `git prune --expire=now` after move/encrypt/wire (may drop unreachable objects other features could want, e.g. a future undo).
3. Pending action field is `file_id` (an `id` field clashes with the flattened item id).
4. Template changes (`value`, new `file`) only reach repositories created by this version.
5. `Resolver` now holds a `Store` clone and the session identity; `reverse` takes an `is_secret` closure so error messages never print secret text.

## Test hook (debug builds only)

`DOTWARDEN_TEST_PASSPHRASE`, `_NEW_PASSPHRASE`, `_RECOVERY_CODE`, `_SECRET_VALUE`, `_WORK_FACTOR` are read inside `#[cfg(debug_assertions)]` blocks in `vault.rs`; they do not exist in release builds. Without them, input needs a TTY on stdin and stderr and no `DOTWARDEN_AGENT`.

## Threat model

Protects the repository, outputs, logs and pending/state files from values. Not protected: plaintext in HOME, same-user processes (can read the session, process environment of chezmoi/helper, or HOME). The token is a guard against stray helper calls, not a boundary. Secret names, override targets and file ids are visible in git. Git history from before `encrypt`/move keeps old plaintext.

## Tests

`tests/vault.rs` (8, real chezmoi/git, two machines): lifecycle/passwd/recover; refusal from args/env/pipe/agent; agent sees no value in 7 commands × json/human, helper refuses without token; two machines with a `machine:srv` override, locked skip/exit 4; edited secret masked → staged into vault; encrypted SSH key (0600, no plaintext in files/git objects); detection → pending → no plaintext in git; canary over ~18 commands (json, human, agent, locked) plus every file under state/, runtime dir, repo tree and git objects. `tests/tui_vault.rs` (2): masked prompt and reveal/lock; `E`.
Hand-broken (both caught, reverted): token check removed (1 failed); masked diff disabled (2 failed).

## Review fixes

1. Detected secrets are never written: `track`/`stage` inspect HOME first. No vault: refused (exit 1; `--allow-plaintext`, humans only). Vault: the file is stored sanitized at once (chezmoi adds a masked copy in a throwaway destination; `secret_move` items remain only for already stored files), so no blob exists to prune. Deviation: the move is applied directly, not proposed.
2. `values wire` always compared the real value; the message now says "hidden" and that a HOME-only value needs `dw stage` first (it was the cause of the review's failure).
3. `values add-secret` has `--apply` (the default, since values cannot wait in pending) and a structured `--dry-run`.
4. Templates carry `templates = N` in `dotwarden.toml`; `dw upgrade-templates` adds a pending item; status and doctor hint. Content-compared, stored files unchanged.
5. `values remove NAME [--force]` (force needs human approval), `values unset-secret NAME --target T` (needs unlock), `values wire PATH NAME --literal TEXT` for placeholders; a placeholder renders empty with a stderr warning, needs no unlock; doctor lists referenced placeholders.

## Gaps

- No `values unset` for secrets, no secret removal; wiring a placeholder is not possible.
- Archive export, private-key block detection beyond PEM headers, multi-line secrets.
- Linux Secret Service path is compile-only. Linux not run.
- TUI vault tests take ~20 s (settle polling).
