# Fresh rewrite replacing dot v1

Dot v1 grew around plan/execute reviews, automatic-scope policies, report branches and a migration engine, and it assumed the user's own chezmoi source. The product goal changed to a lazygit-style, self-contained multi-machine tool with a secrets vault, so v2 starts from an empty `v2` branch rather than evolving v1. Nothing is migrated: users create a fresh repository and agents re-track files from HOME through the CLI. v1 code, docs, ADRs and issues remain in history on `main` (issues labelled `v1-retired`).
