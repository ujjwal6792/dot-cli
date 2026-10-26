# Implementation rules

- Test on macOS only. Do not run Linux, Docker or emulated test runs; the owner
  verifies Linux on real machines after development. Linux-specific code must
  still compile and have unit tests that run on macOS.
- Never run dotwarden or tests against the real HOME; use isolated temporary
  HOME and XDG directories.
- Keep `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` green.
- Follow `docs/spec.md`, `GLOSSARY.md` terms and `docs/adr/`. Report any
  deviation instead of diverging silently.
- Work in this checkout on a branch named after the ticket (`git switch -c t4 v2`).
  No extra worktrees.

## Token budget

- Never print large output. Pipe builds and tests through a filter:
  `cargo test -q 2>&1 | grep -E '^test result|FAILED|panicked' | head -40`.
  Show full output only for the one failing test (`cargo test NAME -- --nocapture | tail -60`).
- Run only the tests you touched while iterating; run the full suite once before
  each commit.
- Do not read `target/`, `libexec/`, `Cargo.lock`, generated files, chezmoi state
  or whole JSON dumps. Use `grep -n`, `head`, or read files in small ranges.
  Read a file once; do not re-read unchanged files.
- Before writing a test, decide what behaviour it proves and how it would fail
  if the feature broke. One behaviour per test; no tests of constants, env-map
  contents or code that cannot fail. Prefer a few end-to-end tests with real
  chezmoi/git over many shallow ones.
- If a test fails twice for an unclear reason, stop and record it in the report
  instead of looping. Do not do broad mutation checks; break a feature by hand
  only for the 2–3 riskiest behaviours of a ticket.
- Keep the ticket report (`docs/tN-report.md`) under 60 lines: built, decisions,
  test summary, gaps.
