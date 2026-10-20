# Default exclusions

Dot installs editable rules in the connected repository's `.chezmoiignore`
(or existing `.chezmoiignore.tmpl`) and `.gitignore`. Files → All means all
**non-ignored** entries. Folder previews in Details use the same native rules.
Capture also respects those rules. Nested rules, templates and explicit `!`
exceptions are evaluated by chezmoi, not by a second approximate matcher.

The complete fresh-install catalogs are checked in here:

- [chezmoi target-path rules](../assets/defaults/.chezmoiignore)
- [Git source-path rules](../assets/defaults/.gitignore)

Git rules include ordinary filenames and common chezmoi-encoded source names.
HOME-rooted exclusions also account for a relocated `.chezmoiroot` at runtime.

## What is excluded

| Category | Examples |
|---|---|
| macOS/Linux clutter | `.DS_Store`, AppleDouble files, `.Trash`, `.trash`, `.Trashes`, `.Trash-*`, desktop metadata |
| Personal data and workspaces | Library, Downloads, Desktop, Documents, media directories, projects, workspaces, notes and journals |
| Installed tools and packages | `.android`, `.bob`, `.bundle`, `.cargo`, `.rustup`, `.bun`, `.nvm`, `.gem`, `.cocoapods`, `.expo`, `.ollama`, `.docker` |
| Dependencies and build output | `node_modules`, `target`, `build`, `dist`, virtual environments, Gradle, Swift/Dart output, frontend caches |
| Runtime state | `.cache`, `.local/state`, named tool data under `.local/share`, shell/REPL histories, logs, sockets and PID files |
| Agent/editor generated data | Codex sessions and plugin caches, Claude project sessions and histories, Gemini temporary files, Zed conversations and embeddings, installed editor extensions |
| Local identities | Named auth/token files, SSH identities and host history, cloud credential caches and local keyrings |

The October 2026 local inventory used directory and filename metadata; it did
not read credentials or personal document contents. It informed the root-only
rules for installed tools and runtime folders. Rules such as `projects/` are
HOME-rooted so an app's configuration subfolder with that name stays eligible.

## What remains available

Dot does not blanket-ignore `.config`, `.agents`, `.codex`, `.claude` or
`.local/share`. Skills, hooks, shell RC files, `.gitconfig`, editor settings,
`.ssh/config`, custom scripts in `bin`/`.local/bin`, themes, and package lockfiles
remain eligible unless another rule excludes them. A theme checkout named
`ghostty` was kept because its name alone does not identify disposable data.

The requested whole-directory exclusions `.cargo`, `.bundle`, `.android` and
`.bob` can contain useful configuration as well as runtime data. They are hidden
by default; narrow those rules if you deliberately want to save a file there.
Do the same for any project or data directory you choose to manage.

## Editing and upgrades

Use **i Ignore** on a file/folder to review updates to both rule files and stop
Git tracking its saved copies while leaving all files on disk.

Setup installs the defaults. Existing setups can select **Actions → Add
developer ignore defaults to both files**. The v2 machine profile is an additive
upgrade: it preserves edited v1 rules and custom rules. Re-running setup does not
regenerate an already-installed profile. Existing Git-tracked files are not
silently untracked by installing defaults; use Ignore to review that change.

Edit the native files to narrow defaults or add exceptions. Chezmoi exceptions
begin with `!` and take precedence; include parent folders as needed. Git has its
own exception and parent-directory semantics. Press **R** to refresh the view
after editing rules externally. Recovery can restore a reviewed ignore operation
when no subsequent changes have occurred.
