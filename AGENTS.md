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
