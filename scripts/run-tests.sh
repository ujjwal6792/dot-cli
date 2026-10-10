#!/usr/bin/env bash
# Public test runner: bats partitions + Go browser tests + Python checks + gitleaks.
# Isolated fixtures only: synthetic HOME/private checkouts, local remotes, mocked
# managers. No host package/config operations, no Go network target required
# beyond the module cache. No releases/tags or actual setup.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PART="${1:-all}"

run_bats() {
  bats "$@"
}

case "$PART" in
  contract)
    run_bats tests/engine-contract.bats tests/installed-engine.bats tests/installed-adapters.bats tests/layout.bats
    ;;
  cli)
    run_bats tests/dot.bats tests/dot-ui.bats tests/intent.bats tests/scope.bats \
      tests/safe-adoption.bats tests/unsafe-adoption.bats tests/tracking-ui.bats \
      tests/layout.bats tests/engine-contract.bats tests/installed-engine.bats \
      tests/installed-adapters.bats
    ;;
  browser)
    run_bats tests/browser-dispatch.bats tests/browser-safety.bats
    if command -v tmux >/dev/null 2>&1; then
      run_bats tests/browser-approval.bats tests/browser-ui.bats
    else
      echo 'skip tmux browser UI tests (tmux unavailable)' >&2
    fi
    (cd tools/dot/browser && go test ./...)
    python3 tests/browser-api-test.py
    ;;
  bootstrap)
    run_bats tests/bootstrap.bats tests/bootstrap-arch.bats tests/bootstrap-ubuntu.bats \
      tests/bootstrap-retirement.bats tests/split-bootstrap.bats
    if command -v tmux >/dev/null 2>&1; then
      run_bats tests/bootstrap-ui.bats
    else
      echo 'skip tmux bootstrap UI tests (tmux unavailable)' >&2
    fi
    ;;
  installed)
    run_bats tests/independent-updates.bats tests/installed-migration.bats \
      tests/installed-engine.bats tests/installed-adapters.bats \
      tests/installed-browser-update.bats tests/init.bats
    python3 tests/tracking-cli-test.py
    ;;
  packaging)
    run_bats tests/public-export.bats tests/release-packaging.bats
    ;;
  python)
    python3 tests/browser-api-test.py
    python3 tests/tracking-cli-test.py
    python3 tests/packages-test.py
    ;;
  go)
    (cd tools/dot/browser && go test ./...)
    ;;
  all)
    "$0" python
    "$0" go
    "$0" contract
    "$0" cli
    "$0" browser
    "$0" bootstrap
    "$0" installed
    "$0" packaging
    run_bats tests/intent-ui.bats tests/sync-skills.bats tests/packages.bats
    if command -v gitleaks >/dev/null 2>&1; then
      gitleaks dir --redact .
    else
      echo 'skip gitleaks (unavailable)' >&2
    fi
    ;;
  *)
    echo "unknown partition: $PART (contract|cli|browser|bootstrap|installed|packaging|python|go|all)" >&2
    exit 2
    ;;
esac
