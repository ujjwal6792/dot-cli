#!/bin/sh
# Tools-only release installer. It never installs runtimes or config dependencies.
set -eu
umask 077

usage() {
  cat <<'EOF'
Usage: install.sh --version VERSION --sha256 REVIEWED_SHA256 [options]
Options: --platform darwin-arm64|linux-amd64|linux-arm64 --url HTTPS_URL
         --root ABSOLUTE_PATH --state-dir ABSOLUTE_PATH --help
The default root is ~/.local/share/dot-cli; executable is ~/.local/bin/dot.
Only public CLI files are installed. No config checkout, dependencies or PATH edits.
EOF
}
fail() { echo "dot installer: $*" >&2; exit 1; }
version= platform= url= digest= root= state_dir= self_update=no yes=no
while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --self-update) self_update=yes; shift ;;
    --yes) yes=yes; shift ;;
    --version|--platform|--url|--sha256|--root|--state-dir)
      [ "$#" -ge 2 ] || fail "missing value for $1"
      key=$1; value=$2; shift 2
      case "$key" in
        --version) version=$value ;; --platform) platform=$value ;; --url) url=$value ;;
        --sha256) digest=$value ;; --root) root=$value ;; --state-dir) state_dir=$value ;;
      esac ;;
    *) fail "unknown option: $1" ;;
  esac
done
[ -n "$version" ] && [ -n "$digest" ] || { usage >&2; exit 2; }
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$' || fail 'invalid semantic version'
case "$version" in *..*) fail 'invalid semantic version';; esac
case "$digest" in *[!a-f0-9]*|'') fail 'sha256 must be 64 lowercase hex digits' ;; esac
[ "${#digest}" -eq 64 ] || fail 'sha256 must be 64 lowercase hex digits'
if [ -z "$platform" ]; then
  os=$(uname -s); arch=$(uname -m)
  case "$os:$arch" in Darwin:arm64|Darwin:aarch64) platform=darwin-arm64 ;;
    Linux:x86_64|Linux:amd64) platform=linux-amd64 ;;
    Linux:aarch64|Linux:arm64) platform=linux-arm64 ;;
    *) fail "unsupported platform: $os/$arch";; esac
fi
case "$platform" in darwin-arm64|linux-amd64|linux-arm64) ;; *) fail 'unsupported platform' ;; esac
case "$url" in '') url="https://github.com/ujjwal6792/dots-cli/releases/download/v$version/dot-cli-$version-$platform.tar.gz";; esac
case "$url" in https://*|file://*) ;; *) fail 'URL must use HTTPS (file:// is test-only)' ;; esac
if [ -z "$root" ]; then root="${HOME:?HOME is required}/.local/share/dot-cli"; fi
case "$root" in /*) ;; *) fail '--root must be absolute' ;; esac
if [ -z "$state_dir" ]; then state_dir="${XDG_STATE_HOME:-${HOME:?HOME is required}/.local/state}/dot"; fi
case "$state_dir" in /*) ;; *) fail '--state-dir must be absolute' ;; esac
case "$root" in *'/../'*|*/..|*'/./'*|*'//'*) fail '--root must be normalized' ;; esac
case "$state_dir" in *'/../'*|*/..|*'/./'*|*'//'*) fail '--state-dir must be normalized' ;; esac
case "$root" in *[![:print:]]*) fail '--root must not contain control characters' ;; esac
case "$root" in
  /|"$HOME"|/usr|/usr/*|/usr/local|/usr/local/*|/opt/homebrew|/opt/homebrew/*|/var/lib/pacman|/var/lib/pacman/*)
    fail 'system/package-manager prefix is protected; use brew upgrade or pacman -Syu for managed installs' ;;
esac

hash_file() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  else fail 'need shasum or sha256sum'; fi
}
replace_pointer() {
  if [ "$(uname -s)" = Darwin ]; then mv -fh "$1" "$2"; else mv -Tf "$1" "$2"; fi
}
check_path() {
  path=$1; rest=$path; current=
  while [ -n "$rest" ]; do
    part=${rest%%/*}; if [ "$rest" = "$part" ]; then rest=; else rest=${rest#*/}; fi
    [ -n "$part" ] || continue
    current="$current/$part"
    [ ! -L "$current" ] || fail "symlink path component refused: $current"
  done
}
check_path "$root"; check_path "$state_dir"; check_path "${HOME:?HOME is required}/.local/bin"
case "$state_dir" in *[![:print:]]*) fail '--state-dir must not contain control characters' ;; esac
json_escape() { printf %s "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }
root_json=$(json_escape "$root")
engine_path_json=$(json_escape "$root/current/tools/dot")
state_dir_json=$(json_escape "$state_dir")
record="$state_dir/install.json"
launcher="${HOME}/.local/bin/dot"
connection="$state_dir/connection.json"
[ ! -L "$record" ] || fail 'installation record symlink is refused'
[ ! -L "$connection" ] || fail 'connection record symlink is refused'
[ ! -L "$root/versions" ] || fail 'versions directory symlink is refused'
old_target=
if [ -L "$root/current" ]; then
  old_target=$(readlink "$root/current")
  case "$old_target" in versions/*) ;; *) fail 'current pointer is outside installer-owned versions';; esac
  case "$old_target" in *..*|/*) fail 'unsafe current pointer';; esac
  [ -d "$root/$old_target" ] || fail 'current pointer target is missing'
elif [ -e "$root/current" ]; then fail 'current path is not an installer-owned symlink'; fi
if [ -f "$record" ]; then
  if grep -q '"kind"[[:space:]]*:[[:space:]]*"package-manager"' "$record"; then
    if grep -q '"manager"[[:space:]]*:[[:space:]]*"brew"' "$record"; then fail 'package-manager-owned install; run brew upgrade dots-cli'; fi
    if grep -q '"manager"[[:space:]]*:[[:space:]]*"pacman"' "$record"; then fail 'package-manager-owned install; run pacman -Syu'; fi
    fail 'unknown package-manager owner; refusing to modify it'
  fi
  grep -q '"kind"[[:space:]]*:[[:space:]]*"artifact"' "$record" || fail 'unknown install owner; refusing to modify it'
  grep -q '"manager"[[:space:]]*:[[:space:]]*"curl"' "$record" || fail 'unknown artifact manager; refusing to modify it'
  record_keys=$(grep -Eo '"[A-Za-z0-9_]+"[[:space:]]*:' "$record" | wc -l | tr -d ' ')
  [ "$record_keys" -eq 12 ] || [ "$record_keys" -eq 13 ] || fail 'malformed or unsupported artifact record; refusing to modify it'
  for field in record_version owner kind manager artifact repository release_tag platform sha256 install_root engine_path browser_sha256; do
    grep -Eq '"'"$field"'"[[:space:]]*:' "$record" || fail 'incomplete artifact record; refusing to modify it'
  done
  if grep -Eq '"record_version"[[:space:]]*:[[:space:]]*2([,}])' "$record"; then
    [ "$record_keys" -eq 13 ] || fail 'malformed v2 artifact record; refusing to modify it'
    grep -Fq '"state_dir":"'"$state_dir_json"'"' "$record" || fail 'recorded state directory mismatch; refusing to modify it'
  else
    grep -Eq '"record_version"[[:space:]]*:[[:space:]]*1([,}])' "$record" && [ "$record_keys" -eq 12 ] || fail 'unsupported artifact record version'
  fi
  grep -Fq '"repository":"ujjwal6792/dots-cli"' "$record" || fail 'unknown recorded repository'
  grep -Eq '"platform"[[:space:]]*:[[:space:]]*"(darwin-arm64|linux-amd64|linux-arm64)"' "$record" || fail 'unsupported recorded platform'
  grep -Eq '"release_tag"[[:space:]]*:[[:space:]]*"v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?"' "$record" || fail 'invalid recorded release tag'
  grep -Eq '"sha256"[[:space:]]*:[[:space:]]*"[a-f0-9]{64}"' "$record" || fail 'invalid recorded archive checksum'
  grep -Eq '"browser_sha256"[[:space:]]*:[[:space:]]*"[a-f0-9]{64}"' "$record" || fail 'invalid recorded browser checksum'
  grep -Fq '"install_root":"'"$root_json"'"' "$record" || fail 'recorded install root mismatch; refusing to modify it'
  grep -Fq '"engine_path":"'"$engine_path_json"'"' "$record" || fail 'recorded engine path mismatch; refusing to modify it'
fi
[ -z "$old_target" ] || [ -f "$record" ] || fail 'unrecorded current install; refusing to claim or replace it'
if [ -L "$launcher" ]; then fail 'launcher symlink is refused'; fi
if [ -e "$launcher" ] && ! grep -Fq "$root/current/tools/dot/dot" "$launcher"; then
  fail "refusing to replace existing launcher: $launcher"
fi
base=$(dirname "$root")
tmp=$(mktemp -d "${base}/.dot-install.XXXXXX" 2>/dev/null) || {
  mkdir -p "$base"; tmp=$(mktemp -d "$base/.dot-install.XXXXXX"); }
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
archive="$tmp/release.tar.gz"
case "$url" in file://*) cp "${url#file://}" "$archive";; *)
  command -v curl >/dev/null 2>&1 || fail 'curl is required to download the public artifact'
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 "$url" -o "$archive";; esac
[ "$(hash_file "$archive")" = "$digest" ] || fail 'archive checksum mismatch; nothing was extracted'

# Validate every tar entry before extraction. Release archives contain only regular
# files/directories; links and special members are intentionally unnecessary.
archive_size=$(wc -c <"$archive" | tr -d ' ')
[ "$archive_size" -le 536870912 ] || fail 'archive exceeds 512 MiB limit'
tar -tzf "$archive" >"$tmp/members" || fail 'invalid archive'
awk '
  BEGIN { bad=0 }
  { name=$0; while (name ~ /\/$/) sub(/\/$/, "", name); if (name ~ /^\// || name ~ /(^|\/)\.\.(\/|$)/ || name ~ /\\/ || name ~ /[^[:print:]]/) bad=1;
    if (name !~ /^dot-cli(\/|$)/) bad=1;
    if (seen[name]++) bad=1
  }
  END { exit bad || NR == 0 || NR > 5000 }
' "$tmp/members" || fail 'unsafe archive member paths'
tar -tvzf "$archive" >"$tmp/verbose" || fail 'invalid archive metadata'
awk 'substr($0,1,1)!="-" && substr($0,1,1)!="d" {bad=1} substr($0,2,9) ~ /[sStT]/ {bad=1} END{exit bad}' "$tmp/verbose" || fail 'links, special files, or set-id archive members are refused'
mkdir "$tmp/unpack"
tar -xzf "$archive" -C "$tmp/unpack" || fail 'archive extraction failed'
payload="$tmp/unpack/dot-cli"
[ -d "$payload/tools/dot" ] && [ -f "$payload/tools/dot/engine.json" ] &&
[ -x "$payload/tools/dot/dot" ] && [ -x "$payload/bin/dot-browser" ] &&
  [ -x "$payload/install.sh" ] && [ -f "$payload/release.json" ] || fail 'incomplete or incompatible release payload'
for required in dot contract.py init.py engine.json; do [ -f "$payload/tools/dot/$required" ] || fail "missing payload file: $required"; done
if grep -q '"format": 2' "$payload/release.json" && grep -q '"engine_api": 1' "$payload/release.json" && grep -q '"version": "'"$version"'"' "$payload/release.json" && grep -q '"platform": "'"$platform"'"' "$payload/release.json"; then :; else fail 'release identity/format mismatch'; fi
# Artifact self-updates validate connected configuration before promotion.
if [ -f "$connection" ]; then
  command -v python3 >/dev/null 2>&1 || fail 'Python is required to validate connected config before self-update; no files changed'
  private_source=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("private_source") or "")' "$connection") || fail 'invalid connection record; refusing self-update'
  config_path=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("config_path") or "")' "$connection") || fail 'invalid connection record; refusing self-update'
  [ -n "$private_source" ] && [ -n "$config_path" ] && [ -f "$config_path" ] || fail 'recorded private source/config is missing; refusing self-update'
  python3 - "$payload/tools/dot" "$state_dir" "$config_path" "$private_source" <<'PY' || fail 'connected install/config identity is malformed or incompatible; old install is unchanged'
import sys, tomllib
sys.path.insert(0, sys.argv[1])
import install_state
record = install_state.read_install_record(sys.argv[2])
connection = install_state.read_connection(sys.argv[2])
with open(sys.argv[3], 'rb') as stream:
    config = tomllib.load(stream)
if not record or not connection or connection['private_source'] != sys.argv[4]:
    raise SystemExit(1)
if config.get('sourceDir') != sys.argv[4] + '/home':
    raise SystemExit(1)
if config.get('data', {}).get('dotEngine') != record['engine_path']:
    raise SystemExit(1)
PY
  python3 "$payload/tools/dot/contract.py" check --engine "$payload/tools/dot" --source "$private_source" >/dev/null || fail 'candidate CLI is incompatible with connected private config; old install is unchanged'
fi
if [ "$self_update" = yes ]; then
  printf 'Public tooling update: %s -> %s (%s)\n' "${old_target:-not installed}" "$version" "$platform"
  printf 'Verified archive SHA-256: %s\n' "$digest"
  printf 'Private connection/configuration and package managers will not be changed.\n'
  if [ "$yes" != yes ]; then
    printf 'Preview only; rerun with --yes to switch the public artifact.\n'
    exit 0
  fi
fi
browser_digest=$(hash_file "$payload/bin/dot-browser")
mkdir -p "$root/versions" "$state_dir" "${HOME:?HOME is required}/.local/bin"
version_dir="$root/versions/$version-$platform"
[ ! -e "$version_dir" ] || fail "version already exists: $version_dir"
mv "$payload" "$version_dir"
record_tmp="$state_dir/.install.$$.json"
cat >"$record_tmp" <<EOF
{"record_version":2,"owner":{"kind":"artifact","manager":"curl"},"artifact":{"repository":"ujjwal6792/dots-cli","release_tag":"v$version","platform":"$platform","sha256":"$digest"},"install_root":"$root_json","engine_path":"$engine_path_json","state_dir":"$state_dir_json","browser_sha256":"$browser_digest"}
EOF
launcher_tmp="${launcher}.$$"
shell_quote() { printf "'%s'" "$(printf %s "$1" | sed "s/'/'\\\\''/g")"; }
cat >"$launcher_tmp" <<EOF
#!/bin/sh
export DOT_STATE_DIR=$(shell_quote "$state_dir")
exec $(shell_quote "$root/current/tools/dot/dot") "\$@"
EOF
chmod 755 "$launcher_tmp"
old_record="$tmp/old-install.json"
had_record=no
if [ -f "$record" ]; then cp -p "$record" "$old_record"; had_record=yes; fi
rollback_pointer() {
  if [ -n "$old_target" ]; then
    ln -s "$old_target" "$root/.restore.$$" && replace_pointer "$root/.restore.$$" "$root/current"
  else
    rm -f "$root/current"
  fi
  rm -rf "$version_dir"
}
ln -s "versions/$version-$platform" "$root/.current.$$"
replace_pointer "$root/.current.$$" "$root/current" || fail 'could not switch current version'
if ! mv -f "$record_tmp" "$record"; then
  rollback_pointer || :
  rm -f "$record_tmp"
  fail 'record promotion failed; prior version restored'
fi
if ! mv -f "$launcher_tmp" "$launcher"; then
  if [ "$had_record" = yes ]; then
    cp -p "$old_record" "$state_dir/.restore-record.$$" && mv -f "$state_dir/.restore-record.$$" "$record"
  else
    rm -f "$record"
  fi
  rollback_pointer || :
  rm -f "$launcher_tmp"
  fail 'launcher promotion failed; prior version and record restored'
fi
printf 'Installed dot %s (%s) under %s\n' "$version" "$platform" "$root"
printf 'Launcher: %s (add ~/.local/bin to PATH manually if needed)\n' "$launcher"
printf 'No Python, config checkout, dependencies, packages, or startup files were changed.\n'
