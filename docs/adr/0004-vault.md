# Vault: one age identity wrapped by passphrase and recovery code

Secrets are stored in the repository as one age file per secret (names visible, values encrypted to a single X25519 recipient), and whole secret files likewise. The private identity is stored twice, encrypted with the passphrase and with a one-time recovery code. As a result, adding secrets never needs unlock, changing the passphrase rewrites one file, and a lost passphrase is recoverable. Visible names were chosen over a single opaque vault file so git history shows which secret changed and agents can wire references without unlocking. The cost is that secret names are metadata visible to repository readers, which ADR 0006 limits by refusing public remotes.

## Consequences

- Rendering a reference goes through a dotwarden helper called by chezmoi with a per-run token; only an unlocked session can render secrets.
- Plaintext applied to HOME is readable by any process running as the user. Dotwarden guarantees only that the repository, its outputs and its logs never contain values.
