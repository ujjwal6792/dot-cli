#!/usr/bin/env bash
# Interactive adapters only; explicit CLI verbs never source this file.
ui_core() {
  local output code=0
  output="$(mktemp "${TMPDIR:-/tmp}/dot-output.XXXXXX")" || return
  # The spinner owns only animation. Replay stdout AND stderr after it exits,
  # rather than relying on gum's transient output/terminal rendering.
  # shellcheck disable=SC2016 # Variables expand in the child shell.
  gum spin --title "dot $1" -- env DOT_SOURCE="$DOT_SOURCE" DOT_UI_OUTPUT="$output" \
    bash -c 'bash "$@" > "$DOT_UI_OUTPUT" 2>&1' dot "$TOOL_DIR/dot" "$@" || code=$?
  cat "$output"
  rm -f "$output"
  if [ "$code" -ne 0 ]; then
    printf 'dot %s failed (exit %s). Details above.\n' "$1" "$code" >&2
  fi
  return "$code"
}
ui_preview() {
  local output file choice code=0
  local choices=('Continue' 'All changes')
  output="$(ui_core "$@")" || { printf '%s\n' "$output"; return 1; }
  file="$(mktemp "${TMPDIR:-/tmp}/dot-preview.XXXXXX")" || return
  printf '%s\n' "$output" > "$file"
  python3 "$TOOL_DIR/preview.py" summary "$file"
  while IFS= read -r choice; do choices+=("$choice"); done < <(python3 "$TOOL_DIR/preview.py" choices "$file")
  while true; do
    choice="$(gum choose --header 'Inspect changes — q closes the diff' "${choices[@]}")" || break
    [ "$choice" != Continue ] || break
    if [ -n "${NO_COLOR+x}" ]; then
      python3 "$TOOL_DIR/preview.py" show "$file" "$choice" | gum pager || code=$?
    elif command -v bat >/dev/null; then
      python3 "$TOOL_DIR/preview.py" show "$file" "$choice" | bat --language diff --color always --paging never --style plain | gum pager || code=$?
    else
      python3 "$TOOL_DIR/preview.py" color "$file" "$choice" | gum pager || code=$?
    fi
    [ "$code" -eq 0 ] || break
  done
  rm -f "$file"
  return "$code"
}
ui_input() {
  local value
  value="$(gum input --prompt "$1: " --placeholder "$1")" || return
  [ -n "$value" ] || return 1
  printf '%s\n' "$value"
}
dot_menu() {
  command -v gum >/dev/null || { fail 'interactive mode requires gum; explicit dot VERB calls do not'; return; }
  [ -t 0 ] && [ -t 1 ] || { fail 'interactive mode requires a terminal; use dot VERB'; return; }
  local verb direction path category os_name message name version script
  local scripts=()
  verb="$(gum choose --header 'dot — configuration management' --height 11 status diff push pull add remove update run doctor intent)" || return 0
  gum style --bold "dot $verb"
  case "$verb" in
    intent) dot_intent pick ;;
    status|doctor) ui_core "$verb" ;;
    diff)
      direction="$(gum choose --header 'Diff direction' repo-to-machine machine-to-repo)" || return 0
      ui_preview diff --direction "$direction" ;;
    push)
      ui_preview diff --direction machine-to-repo || return
      message="$(ui_input 'Commit message')" || return 0
      gum confirm 'Commit and push local managed changes?' --default=false || return 0
      ui_core push --message "$message" ;;
    pull)
      ui_preview pull || return
      gum confirm 'Apply reviewed private changes within exact approved scope?' --default=false || return 0
      ui_core pull --yes ;;
    add)
      path="$(ui_input 'Path to adopt')" || return 0
      category="$(gum choose --header 'Classification' shared templated os-only)" || return 0
      if [ "$category" = os-only ]; then
        os_name="$(gum choose --header 'Target OS' darwin linux)" || return 0
        ui_core add --class "$category" --os "$os_name" "$path"
      else
        ui_core add --class "$category" "$path"
      fi ;;
    remove)
      path="$(ui_input 'Path to unmanage')" || return 0
      gum confirm "Unmanage $path (leave live file in place)?" --default=false || return 0
      ui_core remove "$path" ;;
    update)
      name="$(ui_input 'Pin name')" || return 0
      version="$(ui_input 'New version')" || return 0
      gum confirm "Update $name to $version and run installers?" --default=false || return 0
      ui_core update "$name" "$version" ;;
    run)
      for script in "$DOT_SOURCE"/scripts/*.sh; do
        [ -f "$script" ] && [ ! -L "$script" ] || continue
        name="${script##*/}"; scripts+=("${name%.sh}")
      done
      [ "${#scripts[@]}" -gt 0 ] || { fail 'no scripts available'; return; }
      name="$(gum choose --header 'Run script' "${scripts[@]}")" || return 0
      gum confirm "Run $name? Scripts may change this machine." --default=false || return 0
      ui_core run "$name" ;;
  esac
}
