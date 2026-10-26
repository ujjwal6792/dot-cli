# Refuse public remotes

Secret names, variables and the machine registry are readable by anyone who can read the repository, so dotwarden refuses to push to a public remote. Visibility is checked by an anonymous `git ls-remote` on known hosts when a remote is added and before every push. Self-hosted or unverifiable remotes need a one-time human confirmation that agents cannot give. Warning-only was rejected because one mistaken push publishes the metadata permanently.
