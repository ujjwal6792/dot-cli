# Native setup handoff (#17 → #18)

`internal/setup` owns first-run effects and embeds `*tui.NativeEngine`. It reuses
`internal/native` contracts, manifests, machine identities, adoption v3, strict
JSON decoding, path boundaries and recovery records. It is not another policy
engine or subprocess bridge. The returning-user browser and its configuration
approval gates remain intact.

## Common app entry point

```go
model, engine, err := setup.NewApp(ctx, options)
// Handle err, then run tea.NewProgram(model, tea.WithContext(ctx)).Run().
```

Use this same constructor for bare `dot` and `dot init`. It selects the retained
returning-user app only after read-only readiness validation; otherwise it opens
Welcome. Neither construction nor opening a page authorizes writes. The setup
model also has `tui.NewInitModelContext(ctx, engine)` for deliberately entering
setup; `tui.NewInitModel(engine)` remains available. Context cancellation/quit
cancels pending setup subprocesses. Earlier explicitly completed steps are
retained, never deleted to conceal interruption.

The journey is Welcome → Source → Machine → Dependencies → Tracker → Done.
If Git is missing, a prerequisite Dependencies review is offered before source
creation, then returns to Source. Dependencies use Enter to install/continue and
Space to skip into **Setup incomplete**, not Done. Tracker and Replace default
to Cancel. `J/K` scroll tables, tracker review and the named backup path on small
terminals. Done rechecks required work and offers the returning-user menu;
nothing is automatically adopted, committed or pushed.

## Inputs and CLI contract

`Options.BindFlags(*flag.FlagSet)` registers these accepted common input flags:

| Flag | Options field | Meaning |
| --- | --- | --- |
| `--home` | `Home` | Explicit target HOME; otherwise `HOME` |
| `--source` | `Source` | Private Git checkout root |
| `--config` | `Config` | Local unmanaged tracker `.toml` file |
| `--state-dir` | `State` | Isolated machine state and connection directory |
| `--git` | `Git` | Resolved system Git executable; otherwise PATH `git` |
| `--chezmoi` | `Chezmoi` | Resolved dependency executable |
| `--name` | `Name` | Tracker-only fallback when Git user.name is unset |
| `--email` | `Email` | Tracker-only fallback when Git user.email is unset |

Flags bind values only. **`cmd/dot` routing is intentionally owned by #18 and
has not been implemented in this ticket.** #18 registers action flags itself:
`init --check` calls `engine.Check(ctx)` and prints its `old-user`/`new-user`
verdict or a meaningful failure. It must not install dependencies, create any
directories, or conflate an invalid existing checkout with a new user.
Explicit `--url` clone/local-init routing can call `CreateSource`; do not infer
consent to machine selection, tracker connection or adoption from a URL flag.
There is no `--profile personal|work`, hosting API, private engine path, Python,
gum, gh or shell-policy fallback.

Resolution order is explicit input → `DOT_SOURCE`/`DOT_CONFIG` → the connection
in **the selected state directory** → documented defaults. State is explicit
input → `DOT_STATE_DIR` → `$XDG_STATE_HOME/dot` → `$HOME/.local/state/dot`.
Source defaults to `$HOME/.local/share/dot-config`, genuinely inside HOME.
Config defaults to `$XDG_CONFIG_HOME/chezmoi/chezmoi.toml` or
`$HOME/.config/chezmoi/chezmoi.toml`. All paths must be clean and absolute.
Only Darwin's OS-owned `/var`, `/tmp`, `/etc` aliases are normalized; arbitrary
user symlinks are rejected. Custom state never reads another directory's
connection. If an existing tracker belongs to another state, it fails closed:
choose an explicit compatible `--config`, not an inferred replacement.

The shared native boundary now allows Source under HOME, but rejects HOME inside
Source and overlapping Source/State. Track inventory and target policy exclude
the checkout, state and explicit tracker config (including custom tracker paths).
Git receives the explicit HOME and its `.config` directory for local identity;
explicit Git configuration environment overrides remain honored.

## Exported operations

- `setup.New(Options)` constructs an engine read-only; `Options()` returns resolved
  inputs. `Check(ctx)` and `Continue(ctx)` validate Git root, native contract and
  credential-free origin offline. Malformed/conflicting connection records fail
  clearly; they never trigger another installation's path fallback.
- `ValidateURL(url)` accepts HTTPS, `ssh://` and `git@host:path`; rejects embedded
  credentials, queries/fragments and non-network forms without echoing input.
  `CreateSource(ctx, "clone"|"new", url)` uses system Git, verifies the stored
  origin (not an `insteadOf` transport rewrite) and checks the shared contract.
  New creates an empty declarative contract/manifest and `home/.gitkeep`, runs
  only local init/remote-add, and makes no commit, push or remote repository.
  Failed clones are retained for Continue/Replace inspection.
  `CreateSourceTerminal(ctx, action, url, streams)` is the TUI clone boundary:
  Bubble Tea releases the terminal before native Git/SSH/credential-helper
  authentication prompts. Dot never reads those answers. The non-terminal API
  disables Git terminal prompting for scriptable use; configured Git transports
  and credential helpers remain native Git's responsibility.
- `PlanBackup()` is read-only. `Replace(plan, exactBackupPath)` requires a matching
  path and fresh filesystem/Git fingerprint, exclusively reserves a timestamped
  sibling `...backup-<UTC timestamp>/checkout`, then renames the whole checkout.
  It preserves `.git`, dirty and untracked bytes. Symlinks, collisions, changed
  contents/inodes and pending native recovery are rejected. Failed reservations
  are retained; nothing deletes existing data.
- `SuggestMachine(ctx)` reads OS/user/catalog offline, adds a two-character
  lowercase alphanumeric collision suffix and writes nothing. `SaveMachine(ctx,
  id)` normalizes lowercase, rejects edited catalog collisions and delegates empty
  adoption-v3 identity creation to native. Existing identities and other machines
  are immutable. It creates neither tracker nor connection and enrolls no files.
  Unpublished identities in disconnected state directories are not discoverable.
- `PlanTracker(ctx, name, email)` returns a typed `TrackerPlan` with ID, review,
  exact new config text and freshness fingerprint. Existing compatible TOML is
  detected without rewriting it or revealing arbitrary data. Git identity takes
  precedence over supplied tracker-only fallbacks; Git config stays unchanged.
  `ApplyTracker(ctx, plan, exactID, true)` requires explicit consent and a fresh
  plan. New config is published exclusively as mode 0600. Connection is persisted
  only after tracker identity is verified. Interrupted publication is recovered
  by detection and a new consent review, never deletion or pretend success.
- `ReadConnection()` strictly validates version 1 paths and the native adoption
  machine. `Complete(ctx)` rechecks source, adoption, dependencies, tracker,
  connection and absence of pending recovery. It is never a cached success flag.

The retained map-based `SourceEngine`, `MachineEngine` and `SetupEngine` methods
are thin view adapters for these operations. Configuration plan/apply remains
the embedded shared native engine's API.

## Tracker identity and isolation

Tracker root fields are `sourceDir` (contract's home source), `destDir` (explicit
HOME), `cacheDir` and `persistentState` (inside the selected state directory).
`[data]` contains `name`, `email`, `dotMachine` and canonical `dotOS`.
Existing matching tracker bytes, comments and extra machine data are preserved.
Conflicting identities, alternate JSON/YAML siblings, symlinks and legacy
`dotEngine` executable references require explicit review/migration; no private
template or helper executes during detection. Cache/database files are not
created by setup.

## Dependencies and native terminal boundary

`InitDependencies(ctx)` returns missing/incompatible Git and chezmoi rows with
exact manager command and official link, plus a freshness-bound plan. Existing
working tools are not scheduled for reinstall, upgrade, unlink or overwrite;
incompatible existing executables require manual repair. #18 owns consistent
system-Git / pinned packaged-chezmoi resolution and distribution provenance.

`InstallDependencies(ctx, plan, exactID, tui.TerminalStreams)` runs only the
recomputed approved command. Darwin uses `brew install --formula -- ...`;
Linux uses `pacman -S --needed -- ...`, prefixed with native sudo when needed.
No AUR, apt, `--noconfirm`, sudo `-S` or raw-tty/password fallback exists. Brew
must run unprivileged; auto-update, dependent upgrade and cleanup are disabled.
Missing tools are rechecked afterwards: a zero manager exit alone cannot succeed.

Bubble Tea `tea.Exec` releases the terminal and supplies **all three streams**.
Source clone, dependency setup and the retained Packages UI forward those streams to real native
subprocesses; there are no ignored stream setters. Managers operate on the host
installation as its current owner, not an invented target-HOME installation.

The independent package API supports private `packages/Brewfile` (declarative
`brew`/`cask` records only) and `packages/pacman.txt`. `Packages` status/plan/save/
import never installs. Unsupported syntax is preserved but installation-blocking;
unavailable installed state is never guessed to be missing. Saves preserve the
previous exact bytes in `packages/.backups`. Only `InstallPackages(ctx, plan,
exactID, streams)` can install, after its own review/consent and freshness check;
config Apply and non-terminal Packages apply cannot authorize it. No uninstall,
Ruby Brewfile evaluation, AUR or apt integration is provided.

## Evidence and remaining ownership

Go integration tests use real temporary files and system Git for new/existing
users, read-only/offline detection, transport-shaped HTTPS/SSH clone (Git's real
`insteadOf` rewrite to a local fixture), secret-safe failures, backups, identity,
isolated state, tracker consent and the actual model journey through Done.
The child-process stream test deliberately installs **nothing**: even its zero
exit must report required dependencies still missing. It proves stream wiring,
not a real Homebrew/pacman installation. Existing host chezmoi is used, not
overwritten, in the completed Darwin journey.

Current official documentation was consulted for Bubble Tea v2 `Exec`, go-toml
v2, Homebrew and pacman; Context7 discovery exposed no Context7 tool. Linux
runtime execution is explicitly owner-waived; no Docker daemon was started.
Cross-builds do not establish Linux runtime acceptance. Real network hosting/auth
and real privileged manager installation were not exercised. #18 still owns
`cmd/dot` routing, complete help, dependency resolution/pinning and packaged
Darwin acceptance; #19 owns whole-spec/release acceptance. No release-tag
automation was changed or augmented with branch/PR CI.
