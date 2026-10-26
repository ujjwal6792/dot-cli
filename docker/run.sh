#!/bin/sh
# Run the test suite in a distribution container.
# Usage: docker/run.sh ubuntu|arch|cachyos [--platform linux/amd64|linux/arm64]
#
# ubuntu runs natively (arm64 on Apple Silicon). The official Arch Linux and
# CachyOS images are x86-64 only, so they default to linux/amd64 (emulated on
# Apple Silicon); pass --platform to override.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname "$here")
distro=${1:?usage: docker/run.sh ubuntu|arch|cachyos [--platform P]}
shift
platform=""
while [ $# -gt 0 ]; do
  case "$1" in
    --platform) platform=$2; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done

case "$distro" in
  ubuntu) base=ubuntu:24.04; file=Dockerfile.debian-like ;;
  arch) base=archlinux:latest; file=Dockerfile.arch-like; : "${platform:=linux/amd64}" ;;
  cachyos) base=cachyos/cachyos:latest; file=Dockerfile.arch-like; : "${platform:=linux/amd64}" ;;
  *) echo "unknown distribution $distro" >&2; exit 2 ;;
esac
pflag=""
[ -n "$platform" ] && pflag="--platform $platform"
arch_tag=$(printf '%s' "${platform:-native}" | tr '/' '-')
image="dotwarden-test-$distro-$arch_tag"

# shellcheck disable=SC2086
docker build $pflag --build-arg "BASE=$base" -t "$image" -f "$here/$file" "$here"
emulated=""
if [ -n "$platform" ]; then
  case "$(uname -m)/$platform" in
    arm64/linux/arm64|aarch64/linux/arm64|x86_64/linux/amd64) ;;
    *) emulated=1 ;;
  esac
fi

# shellcheck disable=SC2086
docker run --rm $pflag -e "DOTWARDEN_EMULATED=$emulated" \
  -v "$root:/src:ro" \
  -v "dotwarden-cache-$distro-$arch_tag:/cache" \
  "$image"
