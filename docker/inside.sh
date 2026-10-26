#!/bin/sh
# Runs inside the test containers: copy the source (mounted read-only at /src),
# fetch the pinned tools for this architecture, then run fmt, clippy, the whole
# test suite (including the network tests) and the PTY smoke test.
set -eu

echo "== $(. /etc/os-release && echo "$PRETTY_NAME ($ID, ID_LIKE=${ID_LIKE:-})") on $(uname -m)"
echo "== rustc $(rustc --version), glibc $(ldd --version | head -1 | sed 's/.* //')"

mkdir -p /work /cache/libexec /cache/target
cd /src
tar --exclude=./target --exclude=./libexec --exclude=./.git -cf - . | tar -C /work -xf -
cd /work
ln -sfn /cache/libexec /work/libexec
export CARGO_TARGET_DIR=/cache/target

scripts/fetch-tools --dest /cache/libexec
/cache/libexec/chezmoi --version
/cache/libexec/delta --version

# cargo can deadlock under QEMU user-mode emulation (observed on Apple Silicon
# running amd64 images): keep parallelism low and retry a stuck build; finished
# crates stay cached in /cache/target, so a retry resumes where it stopped.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
limit="${BUILD_TIMEOUT:-2400}"
attempt() { # command...
  n=0
  until timeout "$limit" "$@"; do
    n=$((n + 1))
    [ "$n" -lt 4 ] || { echo "== giving up on: $*"; return 1; }
    echo "== retrying ($n): $*"
  done
}

cargo fmt --check
attempt cargo clippy --all-targets -- -D warnings
attempt cargo test --no-run
DOTWARDEN_NET_TESTS=1 timeout 900 cargo test 2>&1 | tee /tmp/test.log | grep -E '^(test result|     Running|failures:|---- )|FAILED|panicked'
if grep -q 'FAILED' /tmp/test.log || [ "$(grep -c '^test result: ok' /tmp/test.log)" -lt 9 ]; then
  echo "== test run failed, timed out or is incomplete"
  exit 1
fi
if [ -n "${DOTWARDEN_EMULATED:-}" ]; then
  # Observed: ratatui's terminal init fails with ENOSYS on a pty under QEMU
  # user-mode emulation, so the real-terminal smoke test only runs natively.
  echo "== PTY smoke test SKIPPED (emulated architecture)"
else
  python3 -I scripts/pty-smoke.py /cache/target/debug/dotwarden
fi
echo "== ALL OK on $(. /etc/os-release && echo "$ID") $(uname -m)"
