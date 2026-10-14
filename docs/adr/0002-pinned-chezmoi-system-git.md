# Private pinned chezmoi, system Git

Release bundles include verified platform-specific chezmoi v2.73.1; Git stays a
system executable so existing system/global SSH, credential and trust configuration
applies. Bundling both would sever that Git integration, while using arbitrary
system chezmoi would expose policy to upstream version changes. The private bundle
never overwrites Homebrew-owned chezmoi; updates require reviewed locks and archives,
and Linux runtime/library compatibility must be established separately from builds.
