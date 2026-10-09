#!/usr/bin/env bash
# create-private-repo.sh — consent-first creation of a NEW private dotfiles
# repository. Fails closed: no remote change without explicit approval.
# Used by the bundled setup-dots skill; never a fallback for denied access.
set -Eeuo pipefail

NAME='' VISIBILITY='private' APPROVE=no

fail() { echo "create-private-repo: $*" >&2; exit "${2:-1}"; }

usage() {
  echo 'usage: create-private-repo.sh --name OWNER/REPO [--visibility private] [--approve]'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="${2:?}"; shift 2 ;;
    --visibility) VISIBILITY="${2:?}"; shift 2 ;;
    --approve) APPROVE=yes; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" 2 ;;
  esac
done

case "$VISIBILITY" in private) ;; *) fail 'only private visibility is permitted' 3 ;; esac
[[ "$NAME" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail '--name must be OWNER/REPO' 2

export GIT_TERMINAL_PROMPT=0

# Access preflight: authenticated-but-unauthorized must stop here, not create.
if ! gh auth status >/dev/null 2>&1; then
  fail 'GitHub authentication required before any repository operation' 4
fi

# Name conflict check via view; distinguished from access-denied.
view_out="$(gh repo view "$NAME" 2>&1)" && view_rc=0 || view_rc=$?
if [ "$view_rc" -eq 0 ]; then
  fail "name conflict: $NAME already exists or is visible; refusing to reuse/rename" 5
fi
case "$view_out" in
  *authoriz*|*denied*|*403*|*Forbidden*)
    fail "access denied when checking $NAME; refusing creation as a fallback" 6 ;;
esac

if [ "$APPROVE" != yes ]; then
  if [ -r /dev/tty ] && [ -w /dev/tty ]; then
    printf 'Create NEW PRIVATE repository %s? Type yes to approve: ' "$NAME" >/dev/tty
    IFS= read -r answer </dev/tty || answer=''
    case "$answer" in yes|YES) ;; *) fail 'consent declined; no remote change' 7 ;; esac
  else
    fail 'interactive consent unavailable; rerun with --approve or from a terminal' 8
  fi
fi

echo "Creating private repository $NAME"
gh repo create "$NAME" --private --clone=false
