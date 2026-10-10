bootstrap_setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  BOOT="$REPO/bootstrap.sh"
  export HOME="$BATS_TEST_TMPDIR/home" TRACE="$BATS_TEST_TMPDIR/trace"
  export FIXTURE_BIN="$BATS_TEST_TMPDIR/bin" FIXTURE_REMOTE="$BATS_TEST_TMPDIR/remote"
  export FIXTURE_ENGINE_REMOTE="$BATS_TEST_TMPDIR/engine-remote"
  export ENGINE_REPOSITORY=fixture/public-engine
  export GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
  unset DOT_SOURCE DOT_CONFIG XDG_CONFIG_HOME ZDOTDIR CHEZMOI_SOURCE_DIR CHEZMOI_CONFIG_FILE
  mkdir -p "$HOME" "$FIXTURE_BIN" "$BATS_TEST_TMPDIR/seed"
  touch "$TRACE"
  # Keep fixture commits/clones deterministic: an auto-packing seed can race
  # with Git's local hardlink optimization. These settings affect fixture HOME only.
  git config --global gc.auto 0
  git config --global maintenance.auto false
  # Public synthetic private seed: minimal config checkout with a fixture
  # entry, synthetic contract and empty catalog. Never archives a real checkout.
  git init -q -b main "$BATS_TEST_TMPDIR/seed"
  python3 "$REPO/tests/helpers/private-config.py" --source "$BATS_TEST_TMPDIR/seed" \
    --fixtures "$REPO/tests/fixtures" --empty
  mkdir -p "$BATS_TEST_TMPDIR/seed/home/dot_config" "$BATS_TEST_TMPDIR/seed/scripts" "$BATS_TEST_TMPDIR/seed/packages"
  printf 'fixture\n' > "$BATS_TEST_TMPDIR/seed/home/dot_config/fixture.conf"
  printf '#!/bin/sh\nexit 0\n' > "$BATS_TEST_TMPDIR/seed/home/run_after_sync-skills.sh.tmpl"
  cp "$REPO/scripts/sync-skills.sh" "$BATS_TEST_TMPDIR/seed/scripts/"
  printf '[pins.fixture]\nversion = "1.0.0"\ncheck = "fixture --version"\ninstall_darwin = "fixture install"\ninstall_linux = "fixture install"\n' > "$BATS_TEST_TMPDIR/seed/packages/pins.toml"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$BATS_TEST_TMPDIR/seed/tools/dot/"
  printf '{"version":1,"entries":{"fixture":{"destination":".config/fixture.conf","source":"home/dot_config/fixture.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"default":["fixture"]}}\n' > "$BATS_TEST_TMPDIR/seed/tools/dot/manifest.json"
  git -C "$BATS_TEST_TMPDIR/seed" add .
  git -C "$BATS_TEST_TMPDIR/seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm initial
  git clone -q --bare --no-local "$BATS_TEST_TMPDIR/seed" "$FIXTURE_REMOTE"
  git --git-dir="$FIXTURE_REMOTE" config --bool core.bare true
  mkdir -p "$BATS_TEST_TMPDIR/engine-seed/tools/dot"
  git init -q -b main "$BATS_TEST_TMPDIR/engine-seed"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/"
  cp "$REPO/tools/dot/engine.json" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/engine.json"
  cp "$REPO/tools/dot/ui.sh" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/ui.sh"
  if [ "${FIXTURE_ENGINE_CONTRACT:-good}" = bad ]; then printf '{}\n' > "$BATS_TEST_TMPDIR/engine-seed/tools/dot/engine.json"; fi
  git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org add .
  git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm initial
  git clone -q --bare --no-local "$BATS_TEST_TMPDIR/engine-seed" "$FIXTURE_ENGINE_REMOTE"
  git config --global user.name 'Fixture User'
  git config --global user.email fixture@example.org
  git config --global commit.gpgsign false
  cat > "$FIXTURE_BIN/uname" <<'SH'
#!/usr/bin/env bash
case "$1" in -s) echo "${FAKE_OS:-Darwin}" ;; -m) echo "${FAKE_ARCH:-arm64}" ;; esac
SH
  cat > "$FIXTURE_BIN/sw_vers" <<'SH'
#!/usr/bin/env bash
echo "${FAKE_MACOS:-15.0}"
SH
  cat > "$FIXTURE_BIN/id" <<'SH'
#!/usr/bin/env bash
echo "${FAKE_UID:-501}"
SH
  cat > "$FIXTURE_BIN/brew" <<'SH'
#!/usr/bin/env bash
echo "brew $*" >> "$TRACE"
case "$1" in
  --version)
    [ "${BREW_MISSING:-no}" != yes ] || [ -e "$HOME/homebrew-installed" ] || exit 1
    echo 'Homebrew 5.0.0' ;;
  --prefix)
    if [ $# -eq 1 ]; then echo "${BREW_PREFIX:-/opt/homebrew}"; else echo "$FIXTURE_BIN/prefix"; fi ;;
  deps)
    if [[ " $* " == *' --missing '* ]]; then
      if [ "${UNEXPECTED_DEPENDENCY:-no}" = yes ]; then echo unexpected-library; fi
      if [ "${BREW_CLOSURE:-no}" = yes ] && [[ " $* " == *' gum '* ]] && [ ! -e "$HOME/bottle-lib-middle" ]; then echo lib-middle; fi
      if [ "${BREW_CLOSURE:-no}" = yes ] && [[ " $* " == *' lib-middle '* ]] && [ ! -e "$HOME/bottle-lib-leaf" ]; then echo lib-leaf; fi
    elif [ "${BREW_CLOSURE:-no}" = yes ]; then
      printf 'lib-leaf\nlib-middle\n'
      if [ "${BREW_DUPLICATE_ROOT:-no}" = yes ]; then printf 'homebrew/core/gum\n'; fi
    fi ;;
  install)
    if [[ " $* " == *' --dry-run '* ]]; then echo 'fixture bottle/dependency plan'; exit 0; fi
    [ "${HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK:-}" = 1 ] || { echo 'implicit dependent updates are still enabled' >&2; exit 8; }
    [ "${HOMEBREW_NO_INSTALL_CLEANUP:-}" = 1 ] || { echo 'implicit cleanup is still enabled' >&2; exit 8; }
    package="${!#}"
    if [ "${UNBOTTLED_PACKAGE:-}" = "$package" ]; then
      if [[ " $* " == *' --force-bottle '* ]]; then echo "no bottle available: $package" >&2; exit 9; fi
      touch "$HOME/source-build"
    fi
    if [ "$package" = lib-middle ] && [ ! -e "$HOME/bottle-lib-leaf" ]; then echo 'would build implicit dependency lib-leaf' >&2; exit 10; fi
    if [ "$package" = gum ] && [ "${BREW_CLOSURE:-no}" = yes ] && [ ! -e "$HOME/bottle-lib-middle" ]; then echo 'would build implicit dependency lib-middle' >&2; exit 10; fi
    [ "${PACKAGE_FAIL:-no}" != yes ] || exit 1
    touch "$HOME/bottle-$package"
    [ "${VERIFY_FAIL:-no}" = yes ] || touch "$HOME/dependency-installed" ;;
esac
SH
  cat > "$FIXTURE_BIN/gh" <<'SH'
#!/usr/bin/env bash
echo "gh $*" >> "$TRACE"
case " $* " in *' --help '*) [ "${GH_CAPABILITY_FAIL:-no}" != yes ] ;; *)
case "$1" in
  --version)
    if [ -e "$HOME/gh-installed" ] || [ -e "$HOME/bottle-gh" ]; then echo 'gh version 2.80.0'; else echo "gh version ${FAKE_GH_VERSION:-2.80.0}"; fi ;;
  api)
    [ "${API_STILL_DENIED:-no}" != yes ] && { [ "${ACCESS_DENIED:-no}" != yes ] || [ -e "$HOME/logged-in" ]; } || exit 1
    printf '{"full_name":"%s","private":%s,"default_branch":"main"}\n' "${FAKE_REPO_IDENTITY:-ujjwal6792/dotfiles}" "${FAKE_REPO_PRIVATE:-true}" ;;
  auth)
    case "$2" in
      login)
        [ -z "${GH_PROMPT_DISABLED:-}" ] || { echo 'browser prompts disabled' >&2; exit 8; }
        [ "${LOGIN_FAIL:-no}" != yes ] || exit 1; touch "$HOME/logged-in" ;;
      switch) [ "${LOGIN_FAIL:-no}" != yes ] || exit 1; touch "$HOME/logged-in" ;;
      setup-git) [ "${CREDENTIAL_FAIL:-no}" != yes ] ;;
    esac ;;
esac
esac
SH
  chmod +x "$FIXTURE_BIN/"*
  export PATH="$FIXTURE_BIN:$PATH"
}

setup() { bootstrap_setup; }

install_dot() { bash "$BOOT" --yes --source-url "$FIXTURE_REMOTE" --engine-url "$FIXTURE_ENGINE_REMOTE" "$@"; }

missing_gum() {
  cat > "$FIXTURE_BIN/gum" <<'SH'
#!/usr/bin/env bash
[ -e "$HOME/dependency-installed" ] || exit 1
echo 'gum version 0.17.0'
SH
  chmod +x "$FIXTURE_BIN/gum"
}

missing_brew() {
  export BREW_MISSING=yes
  cat > "$FIXTURE_BIN/curl" <<'SH'
#!/bin/bash
if [ "$1" = --version ]; then echo 'curl 8.0.0'; exit 0; fi
echo "curl $*" >> "$TRACE"
[ "${DOWNLOAD_FAIL:-no}" != yes ] || exit 22
while [ "$1" != -o ]; do shift; done
printf 'fixture only; never execute as code\n' > "$2"
SH
  cat > "$FIXTURE_BIN/bash" <<'SH'
#!/bin/bash
case "$1" in
  */homebrew.sh)
    echo 'fixture Homebrew provisioning (no live installer execution)' >> "$TRACE"
    [ "${BREW_SETUP_FAIL:-no}" != yes ] || exit 7
    touch "$HOME/homebrew-installed" ;;
  *) exec /bin/bash "$@" ;;
esac
SH
  chmod +x "$FIXTURE_BIN/curl" "$FIXTURE_BIN/bash"
}

deny_git_access() {
  export REAL_GIT="$(command -v git)" GIT_ACCESS_DENIED=yes
  cat > "$FIXTURE_BIN/git" <<'SH'
#!/bin/bash
if [ "$1" = ls-remote ]; then
  if [ "${GIT_STILL_DENIED:-no}" = yes ] || { [ "${GIT_ACCESS_DENIED:-no}" = yes ] && [ ! -e "$HOME/logged-in" ]; }; then exit 128; fi
fi
exec "$REAL_GIT" "$@"
SH
  chmod +x "$FIXTURE_BIN/git"
}

ubuntu_fixture() {
  export FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_DEB_ARCH=amd64
  export REAL_CAT="$(command -v cat)"
  cat > "$FIXTURE_BIN/cat" <<'SH'
#!/bin/bash
if [ "$*" = /etc/os-release ]; then
  printf 'ID=%s\nVERSION_ID="%s"\n' "${FAKE_DISTRO:-ubuntu}" "${FAKE_UBUNTU:-24.04}"
  exit 0
fi
exec "$REAL_CAT" "$@"
SH
  cat > "$FIXTURE_BIN/dpkg" <<'SH'
#!/bin/bash
[ "$*" = --print-architecture ] || exit 9
echo "${FAKE_DEB_ARCH:-amd64}"
SH
  chmod +x "$FIXTURE_BIN/cat" "$FIXTURE_BIN/dpkg"
}

arch_fixture() {
  ubuntu_fixture
  export FAKE_DISTRO=arch
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/bash
echo "sudo $*" >> "$TRACE"
[ "${SUDO_DENIED:-no}" != yes ] || { echo 'fixture sudo permission denied' >&2; exit 1; }
[ "$1" != -n ] || shift
[ "$1" != -v ] || exit 0
[ "$1" = pacman ] || { echo 'unapproved sudo boundary' >&2; exit 9; }
exec "$@"
SH
  cat > "$FIXTURE_BIN/pacman" <<'SH'
#!/bin/bash
echo "pacman $*" >> "$TRACE"
[ "$1" = -Syu ] || { echo 'partial upgrade forbidden by fixture' >&2; exit 9; }
shift
if [ "$1" = --noconfirm ]; then shift; else
  echo 'Packages: fixture-system-upgrade fixture-dependency'
  printf 'Proceed with installation? [Y/n] '
  read -r answer
  [ "$answer" = y ] || exit 1
fi
[ "${PACKAGE_FAIL:-no}" != yes ] || { echo 'fixture network/package failure' >&2; exit 1; }
[ "${VERIFY_FAIL:-no}" != yes ] || exit 0
for package in "$@"; do
  case "$package" in
    core/bash) touch "$HOME/bash-installed" ;;
    core/curl) touch "$HOME/curl-installed" ;;
    core/python) touch "$HOME/python-installed" ;;
    extra/git) touch "$HOME/git-installed" ;;
    extra/github-cli) touch "$HOME/gh-installed" ;;
    extra/chezmoi) touch "$HOME/chezmoi-installed" ;;
    extra/gum) touch "$HOME/dependency-installed" ;;
    *) echo 'nonofficial target forbidden by fixture' >&2; exit 9 ;;
  esac
done
SH
  chmod +x "$FIXTURE_BIN/sudo" "$FIXTURE_BIN/pacman"
}

ubuntu_packages() {
  export REAL_SHA256="$(command -v shasum)"
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/bash
echo "sudo $*" >> "$TRACE"
[ "${SUDO_DENIED:-no}" != yes ] || { echo 'fixture sudo permission denied' >&2; exit 1; }
[ "$1" != -n ] || shift
[ "$1" != -v ] || exit 0
case "$1" in
  env) shift; [ "$1" = DEBIAN_FRONTEND=noninteractive ] || exit 9; shift; exec "$@" ;;
  test)
    [ "$3" != /etc/apt/keyrings ] && [ "$3" != /etc/apt/sources.list.d ] || exit 1
    exec /bin/test "$2" "$HOME/system$3" ;;
  cat) exec "$REAL_CAT" "$HOME/system$2" ;;
  sha256sum) exec sha256sum "$HOME/system$2" ;;
  install)
    [ "${REPOSITORY_FAIL:-no}" != yes ] || { echo 'fixture repository write denied' >&2; exit 1; }
    case "$*" in
      'install -d -m 755 /etc/apt/keyrings /etc/apt/sources.list.d') mkdir -p "$HOME/system/etc/apt/keyrings" "$HOME/system/etc/apt/sources.list.d" ;;
      'install -m 644 '*'/etc/apt/'*) mkdir -p "$(dirname "$HOME/system${!#}")"; cp "$4" "$HOME/system${!#}" ;;
      *) echo 'unapproved sudo boundary' >&2; exit 9 ;;
    esac ;;
  *) echo 'unapproved sudo boundary' >&2; exit 9 ;;
esac
SH
  cat > "$FIXTURE_BIN/apt-get" <<'SH'
#!/bin/bash
echo "apt-get $*" >> "$TRACE"
if [ "$1" = --simulate ]; then
  [ "${APT_SIMULATE_FAIL:-no}" != yes ] || exit 100
  echo 'Inst fixture-dependency (1.0 Ubuntu)'; exit 0
fi
if [ "$1" = update ]; then [ "${APT_UPDATE_FAIL:-no}" != yes ] || exit 100; exit 0; fi
[[ " $* " == *' install '* ]] || exit 9
[ "${PACKAGE_FAIL:-no}" != yes ] || exit 100
[ "${VERIFY_FAIL:-no}" != yes ] || exit 0
for package in "$@"; do
  case "$package" in
    */gum_*.deb) [ "$(cat "$package")" = 'verified fixture artifact' ] || exit 9; touch "$HOME/dependency-installed" ;;
    */chezmoi_*.deb) [ "$(cat "$package")" = 'verified fixture artifact' ] || exit 9; touch "$HOME/chezmoi-installed" ;;
    gh) touch "$HOME/gh-installed" ;;
    python3) touch "$HOME/python-installed" ;;
    curl) touch "$HOME/curl-installed" ;;
    git) touch "$HOME/git-installed" ;;
  esac
done
SH
  cat > "$FIXTURE_BIN/curl" <<'SH'
#!/bin/bash
if [ "$1" = --version ]; then
  [ "${CURL_MISSING:-no}" != yes ] || [ -e "$HOME/curl-installed" ] || exit 1
  echo 'curl 8.0.0'; exit 0
fi
echo "curl $*" >> "$TRACE"
[ "${DOWNLOAD_FAIL:-no}" != yes ] || exit 22
while [ "$1" != -o ]; do shift; done
printf 'verified fixture artifact\n' > "$2"
SH
  cat > "$FIXTURE_BIN/sha256sum" <<'SH'
#!/bin/bash
# Success models an externally verified release; corrupt cases use real SHA256.
if [ "${CHECKSUM_FAIL:-no}" = yes ]; then exec "$REAL_SHA256" -a 256 "$1"; fi
if [ "$(cat "$1")" != 'verified fixture artifact' ]; then exec "$REAL_SHA256" -a 256 "$1"; fi
case "$1" in
  */gum_0.17.0_amd64.deb) hash=4c59b09c7248ea03c1544a11506b1152b1d8bd20e602fb2c2e9d158204d1f490 ;;
  */gum_0.17.0_arm64.deb) hash=9ae145bdd738f65e843f904957496d5aea1c2c182f3f1999b609ed9627f6a68e ;;
  */chezmoi_2.73.0_linux_amd64.deb) hash=24df0be680b422e87fd8b4930fba583650920d68af81d53162a30867f8a8e0c8 ;;
  */chezmoi_2.73.0_linux_arm64.deb) hash=99372f1e46c0532d377c0a368d3d3ba60eb252a9dbba3ca43e1767f571a73362 ;;
  */dot-github-cli.gpg) hash=6084d5d7bd8e288441e0e94fc6275570895da18e6751f70f057485dc2d1a811b ;;
  *) echo 'unexpected checksum boundary' >&2; exit 9 ;;
esac
echo "$hash  $1"
SH
  chmod +x "$FIXTURE_BIN/sudo" "$FIXTURE_BIN/apt-get" "$FIXTURE_BIN/curl" "$FIXTURE_BIN/sha256sum"
}
