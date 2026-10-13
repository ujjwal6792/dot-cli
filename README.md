# dots-cli

One Go CLI for setting up and managing a user's private dotfiles repository.
The target runtime dependencies are **Git and chezmoi**. Go dependencies are
compiled into `dot`; users do not need Go, Python, Bash, gum, or GitHub CLI to
run the application. Native package managers are invoked only for explicitly
requested dependency installation.

## Current development status

The legacy Bash/Python implementation, subprocess bridge, installers, and
release pipeline have been removed intentionally. This checkout is a **Go
rewrite foundation, not a functioning configuration manager or release**.

The Go TUI components and their model tests remain in `internal/tui`. The native
engine and integrated app are pending. `cmd/dot` currently provides help/version
and returns an explicit error for unfinished operations; it has no legacy fallback.

```sh
go run ./cmd/dot --help
go test ./...
go vet ./...
go build -o dot ./cmd/dot
```

Bare `go run ./cmd/dot` will open the app once the native runtime is implemented.
It currently reports that the runtime is unfinished.

## Releases

GitHub Actions runs only when a version tag such as `v1.2.3` is pushed. It builds
`darwin-arm64`, `linux-amd64`, and `linux-arm64` archives, generates `SHA256SUMS`,
and creates a GitHub release. Tags such as `v1.2.3-rc.1` create prereleases.
Each archive currently contains `dot`; Git and chezmoi remain external dependencies.

There are no pull-request or branch-push checks. Agents run tests, vet, and builds
locally before pushing. Release tags publish the tagged code, including its
current implementation status.

## Remaining implementation

[Spec #10](https://github.com/ujjwal6792/dots-cli/issues/10) defines the product.
The remaining issues own native replacements for behavior removed with the old
engine, including work previously implemented for #13–#15:

| Issue | Ownership |
| --- | --- |
| [#16](https://github.com/ujjwal6792/dots-cli/issues/16) | Shared Go configuration engine, machine assignments/local adoption, exact plans and apply, returning-user TUI, machine-2 flow, status/diff/scope JSON |
| [#17](https://github.com/ujjwal6792/dots-cli/issues/17) | Native source setup and machine identity, dependency handling, Tracker/connection state, complete setup journey and `dot init --check` |
| [#18](https://github.com/ujjwal6792/dots-cli/issues/18) | Single CLI integration, complete command routing, Git/chezmoi resolution, Go release packaging and artifact acceptance |
| [#19](https://github.com/ujjwal6792/dots-cli/issues/19) | Whole-spec acceptance, safety checks, help, documentation and ADRs |

See [native rewrite acceptance](docs/native-rewrite.md) for the required behavior
and the historical test reference. Passing retained UI tests does not establish
native engine or release readiness.

## Product boundaries

- One Bubble Tea app for setup and subsequent use; scriptable verbs call the
  same in-process engine.
- User-owned private repository, connected with system Git. Users create their
  remote repository themselves.
- Track and Adopt are separate, exact reviewed operations. Navigation never
  authorizes changes. Unsafe and desktop effects require separate confirmation.
- Machine identity and local adoption state remain local; shared assignments
  change only through an approved Track plan.
- Brew and pacman are the initial dependency-install integrations. AUR helpers,
  apt integration, Windows, and legacy group-scope migration are outside #10.
- Private configuration, credentials, inventories and recovery backups are not
  shipped in public releases.
