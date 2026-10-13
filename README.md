# dots-cli

One Go CLI for setting up and managing a user's private dotfiles repository.
The target runtime dependencies are **Git and chezmoi**. Go dependencies are
compiled into `dot`; users do not need Go, Python, Bash, gum, or GitHub CLI to
run the application. Native package managers are invoked only for explicitly
requested dependency installation.

## Current development status

The legacy Bash/Python implementation, subprocess bridge, installers, and
release scripts have been removed intentionally. The shared native engine,
complete setup app, command router and Go release tooling are now implemented.
Whole-spec acceptance and final user documentation remain owned by #19; this
development checkout is not a tagged release.

`cmd/dot` routes interactive and scriptable operations through the same native
engine. See [native CLI/distribution](docs/native-distribution.md) for command
syntax, retained/retired aliases, packaging and local artifact acceptance.

```sh
go run ./cmd/dot --help
go test ./...
go vet ./...
go build -o dot ./cmd/dot
```

Bare `go run ./cmd/dot` and `go run ./cmd/dot init` open the same app.
Development builds require system Git and chezmoi v2.73.1; packaged binaries
resolve their private pinned chezmoi bundle, never a checkout fallback.

## Releases

GitHub Actions runs only when a version tag such as `v1.2.3` is pushed. It builds
`darwin-arm64`, `linux-amd64`, and `linux-arm64` archives, generates `SHA256SUMS`,
and creates a GitHub release. Tags such as `v1.2.3-rc.1` create prereleases.
Each archive contains `dot`, private pinned chezmoi v2.73.1, its license and
verified release metadata. System Git remains an external dependency.

There are no pull-request or branch-push checks. Agents run tests, vet, and builds
locally before pushing. Release tags publish the tagged code, including its
current implementation status.

## Implementation ownership

[Spec #10](https://github.com/ujjwal6792/dots-cli/issues/10) defines the product.
The issues below own native replacements for behavior removed with the old
engine, including work previously implemented for #13–#15. #16/#17 are merged;
#18 delivers CLI/distribution; #19 owns the remaining whole-spec gate:

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
