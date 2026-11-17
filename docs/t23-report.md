# T23 report: HTTPS remotes through credential helpers

## Built
- `toolchain::is_credential_key`; credential keys join the forwarded global keys.
  `Store` keeps every credential entry (no de-duplication) in config order, empty
  values included. `Store::git_command` passes them; `git_command_anonymous`
  omits them; `git_credential_entries` exposes them.
- `remote::probe_command` (anonymous `ls-remote`) uses `git_command_anonymous`,
  plus the existing `credential.helper=`, `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS`.
- `remote::credential_helper_applies` (url-scoped matching on scheme+host[+path],
  empty value resets). `https_warning(store, url)` is silent when a helper applies.
- Auth-failure hint now suggests `gh auth setup-git` or the SSH URL.
- `dw doctor` line "https credentials" when the default remote is HTTPS.
- chezmoi's git shares the env via `apply_git_env`, so it gets the helpers too.

## Decisions
- Global `credential.username` is not forwarded (only url-scoped, as asked).
- Doctor line is informational (never fails the check).

## Tests (macOS, no network)
- Unit: key filter; helper matching by URL, order, reset, path scope.
- `git_isolation`: entries in order with resets, ignoring alias/url/hooks keys;
  `git credential fill` through dotwarden's git calls a script helper while
  `alias.*` stays blocked and the anonymous command never calls it; probe command
  env has no `credential.*` keys.
- `remote`: warning and doctor line without a helper, none with one.
- Hand-broken: helpers not passed (fill test fails); probe on the normal command
  (probe test fails). Both restored.

## Gaps
- Real `gh`/osxkeychain helpers and a real HTTPS push are not exercised.
- Include files in the global config are followed by git itself, as for identity.
