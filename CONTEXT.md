# Dot workspace

Rust + Ratatui replace the retired Go application. Chezmoi owns source naming,
rendering, diffing, importing and applying; system Git owns repository behavior.
Read [the Rust specification](docs/rust-rewrite-spec.md) for module boundaries,
operation semantics and acceptance coverage. [The interview](docs/tui-redesign-discovery.md)
and [walkthroughs](docs/tui-redesign-walkthroughs.md) record accepted product choices.

`src/backend.rs` adapts chezmoi. `src/operations.rs` owns exact reviews, locks,
backups and execution; CLI and TUI both use it. `src/sync.rs` owns local scheduling
and optional report branches. `src/settings.rs` writes native chezmoi data and
helper templates; it does not interpret arbitrary configuration languages.

Keep HOME drift, source changes, actual Git staging, outgoing commits and dated
machine reports distinct. Empty selections never mean all files. A selected-file
restore never replaces unrelated local enablement. Optional metadata must not
become an alternate file engine. Strict automation is a supported subset, not a
sandbox for repository code. Detailed activity stays local by default.

Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
and `cargo test --locked` with Git and chezmoi v2.73.1 available. Integration tests
use isolated HOME/source/config/state and local Git remotes. Builds do not prove
Linux runtime behavior; the release matrix runs native tests on each platform.
Historical Go acceptance documents describe the pre-rewrite baseline only.
