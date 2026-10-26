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

cargo fmt --check
cargo clippy --all-targets -- -D warnings
DOTWARDEN_NET_TESTS=1 cargo test 2>&1 | tee /tmp/test.log | grep -E '^(test result|     Running|failures:|---- )|FAILED|panicked'
grep -q 'FAILED' /tmp/test.log && exit 1
python3 -I scripts/pty-smoke.py /cache/target/debug/dotwarden
echo "== ALL OK on $(. /etc/os-release && echo "$ID") $(uname -m)"
