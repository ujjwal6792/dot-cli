#!/usr/bin/env bats
# Public release packaging: bootstrap/engine parity, setup-dots consent,
# consent-first private-repo creation with isolated fake gh, secret scans,
# and docs coverage. No real remote actions or credentials involved.
# Public layout: repo root IS the public staging (no public/dots-installer).

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  PUB="$REPO"
  export HOME="$BATS_TEST_TMPDIR/home" TRACE="$BATS_TEST_TMPDIR/trace"
  FIXTURE_BIN="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$HOME" "$FIXTURE_BIN"
  : >"$TRACE"
  cat > "$FIXTURE_BIN/gh" <<'SH'
#!/usr/bin/env bash
echo "gh $*" >> "$TRACE"
case "$1" in
  auth) [ "${GH_AUTH_DENIED:-no}" != yes ] || { echo 'not logged in' >&2; exit 1; } ;;
  repo)
    case "$2" in
      view)
        if [ "${GH_VIEW_ACCESS_DENIED:-no}" = yes ]; then
          echo 'HTTP 403: Forbidden (authorization error)' >&2; exit 1
        fi
        if [ "${GH_VIEW_EXISTS:-no}" = yes ]; then echo '{"name":"taken"}'; exit 0; fi
        echo 'HTTP 404: Not Found' >&2; exit 1 ;;
      create)
        [ "${GH_CREATE_FAIL:-no}" != yes ] || { echo 'name already exists on this account' >&2; exit 1; }
        echo "created $4" ;;
    esac ;;
esac
SH
  chmod +x "$FIXTURE_BIN/gh"
  export PATH="$FIXTURE_BIN:$PATH" GIT_TERMINAL_PROMPT=0
  CREATE="$PUB/scripts/create-private-repo.sh"
}

@test "public bootstrap is executable and documents split install defaults" {
  [ -x "$PUB/bootstrap.sh" ]
  grep -q 'dot-cli' "$PUB/bootstrap.sh"
  grep -q 'dot-config' "$PUB/bootstrap.sh"
}

@test "public engine matches reviewed reusable code and excludes private catalogs" {
  for file in dot contract.py engine.json gitconfig.py init.py init-python-bootstrap.sh packages.py intent.py layout.py migration.py picker.py preview.py scope.py skills.py state.py ui.sh; do
    [ -f "$PUB/tools/dot/$file" ]
  done
  [ ! -e "$PUB/tools/dot/manifest.json" ]
  [ ! -e "$PUB/tools/dot/groups.json" ]
  [ ! -e "$PUB/dot-contract.json" ]
  [ ! -e "$PUB/home" ]
}

@test "setup-dots bundles consent-first private creation and separate Track/Adopt rounds" {
  SKILL="$PUB/skills/setup-dots/SKILL.md"
  [ -f "$SKILL" ]
  grep -q 'Download, review, execute\|download/review/execute' "$SKILL"
  grep -qi 'explicit consent' "$SKILL"
  grep -q 'no remote change' "$SKILL"
  grep -qi 'never.*fallback\|never use creation as a fallback' "$SKILL"
  grep -qi 'distinct Track and Adopt\|Track.*Adopt' "$SKILL"
  grep -qi 'no private configuration\|no private' "$SKILL"
}

@test "create-private-repo: approved typed consent creates a private repo" {
  run bash "$CREATE" --name fixture/dots --approve
  [ "$status" -eq 0 ]
  grep -q 'gh repo create fixture/dots --private --clone=false' "$TRACE"
}

@test "create-private-repo: unapproved noninteractive run makes no remote change" {
  run bash "$CREATE" --name fixture/dots </dev/null
  [ "$status" -ne 0 ]
  ! grep -q 'gh repo create' "$TRACE"
}

@test "create-private-repo: typed refusal makes no remote change" {
  if command -v script >/dev/null 2>&1; then
    rc=0; printf 'no\n' | script -q /dev/null bash "$CREATE" --name fixture/dots >/dev/null 2>&1 || rc=$?
    [ "$rc" -ne 0 ]
    ! grep -q 'gh repo create' "$TRACE"
  else
    skip 'pty harness unavailable'
  fi
}

@test "create-private-repo: access-denied view refuses creation as a fallback" {
  export GH_VIEW_ACCESS_DENIED=yes
  run bash "$CREATE" --name fixture/dots --approve
  [ "$status" -eq 6 ]
  ! grep -q 'gh repo create' "$TRACE"
  unset GH_VIEW_ACCESS_DENIED
}

@test "create-private-repo: name conflict stops before creation" {
  export GH_VIEW_EXISTS=yes
  run bash "$CREATE" --name fixture/dots --approve
  [ "$status" -eq 5 ]
  ! grep -q 'gh repo create' "$TRACE"
  unset GH_VIEW_EXISTS
}

@test "create-private-repo: unauthenticated gh stops before any repo call" {
  export GH_AUTH_DENIED=yes
  run bash "$CREATE" --name fixture/dots --approve
  [ "$status" -eq 4 ]
  ! grep -q 'gh repo view' "$TRACE"
  unset GH_AUTH_DENIED
}

@test "create-private-repo: non-private visibility and bad names are rejected" {
  run bash "$CREATE" --name fixture/dots --visibility public --approve
  [ "$status" -eq 3 ]
  run bash "$CREATE" --name 'not a name' --approve
  [ "$status" -eq 2 ]
  ! grep -q 'gh repo create' "$TRACE"
}

@test "create-private-repo: gh create failure propagates as failure" {
  export GH_CREATE_FAIL=yes
  run bash "$CREATE" --name fixture/dots --approve
  [ "$status" -ne 0 ]
  unset GH_CREATE_FAIL
}

@test "public tree contains no token shapes or private-history artifacts" {
  ! grep -rInE 'ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|AKIA[0-9A-Z]{16}' "$PUB/tools" "$PUB/scripts" "$PUB/skills" "$PUB/tests/fixtures"
  ! find "$PUB" -path "$PUB/.git" -prune -o \( -name '.git' -o -name '*.pack' -o -name 'secrets.env' \) -print | grep .
}

@test "gitleaks passes on the public tree" {
  if command -v gitleaks >/dev/null 2>&1; then
    run gitleaks dir --redact "$PUB"
    [ "$status" -eq 0 ]
  else
    skip 'gitleaks unavailable'
  fi
}

@test "public README covers split defaults init consent and prebuilt platform releases" {
  R="$PUB/README.md"
  for needle in 'ujjwal6792/dots-cli' '~/.local/share/dot-cli' '~/.local/share/dot-config' '--existing update|keep' 'Apple Silicon' 'Ubuntu' 'Arch' 'full system upgrade' \
    'empty adoption scope' 'neither checkout' 'detected read-only' 'skills.bak' 'missing tools' '--yes' '--replace-chezmoi' \
    'recovery' 'Unsupported' 'dot init' 'does not install dependencies or create a chezmoi tracker' 'Go' 'skills/dot/SKILL.md'; do
    grep -qi -e "$needle" "$R" || { echo "missing: $needle"; return 1; }
  done
  grep -q 'curl' "$R"
}

@test "export scripts enforce the public-only boundary" {
  run python3 "$REPO/scripts/export-public.py" --output "$BATS_TEST_TMPDIR/export-check"
  [ "$status" -eq 0 ]
  [ ! -e "$BATS_TEST_TMPDIR/export-check/tools/dot/manifest.json" ]
  [ ! -e "$BATS_TEST_TMPDIR/export-check/tools/dot/groups.json" ]
  [ -f "$BATS_TEST_TMPDIR/export-check/tools/dot/engine.json" ]
}
