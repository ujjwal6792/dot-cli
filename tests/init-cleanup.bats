#!/usr/bin/env bats
load bootstrap-helper

setup() {
  bootstrap_setup
  cat > "$FIXTURE_BIN/chezmoi" <<'SH'
#!/bin/bash
echo "chezmoi $*" >> "$TRACE"
[ "$*" = --version ] || exit 90
echo 'chezmoi version fixture'
SH
  chmod +x "$FIXTURE_BIN/chezmoi"
  install_dot --yes
  : > "$TRACE"
}

@test "init rejects the removed personal profile flag" {
  run "$HOME/.local/bin/dot" init --profile personal --check
  [ "$status" -ne 0 ]
}

@test "init rejects the removed work profile flag" {
  run "$HOME/.local/bin/dot" init --profile work --check
  [ "$status" -ne 0 ]
}

@test "noninteractive init creates a tracker with no profile key" {
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  [ -f "$HOME/.config/chezmoi/chezmoi.toml" ]
  run grep -F 'profile' "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -ne 0 ]
  grep -F 'name = "Fixture User"' "$HOME/.config/chezmoi/chezmoi.toml"
}

@test "dot help lists the full verb list" {
  run "$HOME/.local/bin/dot" -h
  [ "$status" -eq 0 ]
  [[ "$output" != *'personal|work'* ]]
  for verb in status diff push pull add remove run update doctor scope intent layout; do
    [[ "$output" == *"$verb"* ]]
  done
}

@test "init with missing source and no URL stops with guidance and assumes nothing" {
  rm -rf "$HOME/.local/share/dot-config"
  run "$HOME/.local/bin/dot" init --source "$HOME/.local/share/dot-config" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -eq 0 ]
  [[ "$output" == *'--url'* ]]
  [ ! -e "$HOME/.local/share/dot-config" ]
  ! grep -E '^gh api|^gh repo clone' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "init clones an explicit URL with plain git and no hardcoded owner" {
  rm -rf "$HOME/.local/share/dot-config"
  run "$HOME/.local/bin/dot" init --url "$FIXTURE_REMOTE" --source "$HOME/.local/share/dot-config" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -eq 0 ]
  [ -d "$HOME/.local/share/dot-config/.git" ]
  ! grep -E '^gh api|^gh repo clone' "$TRACE"
  [[ "$(git -C "$HOME/.local/share/dot-config" remote get-url origin)" == *"$FIXTURE_REMOTE"* ]]
}

@test "init rejects a credential-bearing URL without cloning" {
  rm -rf "$HOME/.local/share/dot-config"
  run "$HOME/.local/bin/dot" init --url 'https://user:secret@example.invalid/owner/repo.git' --source "$HOME/.local/share/dot-config" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential'* ]]
  [ ! -e "$HOME/.local/share/dot-config" ]
}

@test "init never mentions a hardcoded owner repository" {
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" != *'ujjwal6792/dotfiles'* ]]
  run grep -rn 'ujjwal6792/dotfiles' "$REPO/tools/dot/init.py" "$REPO/tools/dot/dot" "$REPO/tools/dot/init-python-bootstrap.sh"
  [ "$status" -ne 0 ]
}

@test "init source contains no gum curses or raw tty prompt path" {
  run grep -nwE 'gum|curses' "$REPO/tools/dot/init.py"
  [ "$status" -ne 0 ]
  run grep -nE '/dev/tty|picker|[^_a-zA-Z](ask|consent)\(' "$REPO/tools/dot/init.py"
  [ "$status" -ne 0 ]
  run grep -nE '/dev/tty' "$REPO/tools/dot/init-python-bootstrap.sh"
  [ "$status" -ne 0 ]
}
