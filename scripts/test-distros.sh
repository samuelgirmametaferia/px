#!/usr/bin/env bash
# Live-test the Debian/Fedora/openSUSE recipes in docker containers.
# Requires: docker daemon running (sudo systemctl start docker) + the
# release binary built (cargo build --release).
#   usage: ./scripts/test-distros.sh
set -euo pipefail
BIN="$(pwd)/target/release/px"
[ -x "$BIN" ] || { echo "build first: cargo build --release"; exit 1; }

run_in() {  # image, install-cmd(s)
  local image="$1"; shift
  echo "── $image ─────────────────────────────────"
  docker run --rm -v "$BIN:/usr/local/bin/px:ro" "$image" sh -c "$*" 2>&1 | tail -4
  echo
}

# Debian: px auto-detects debian → apt; install a tiny package + analyze a project
run_in debian:bookworm '
  apt-get update -qq &&
  px doctor | grep -E "active|sandbox" &&
  px --dry-run install sl | grep "\[dry-run\]" &&
  px --dry-run --global install for /tmp 2>/dev/null | head -3 ||
  px --dry-run install sl | grep "\[dry-run\]"'

# Fedora: dnf + a small install
run_in fedora:latest '
  px doctor | grep -E "active|sandbox" &&
  px --dry-run install sl | grep "\[dry-run\]"'

# openSUSE: zypper
run_in opensuse/leap:latest '
  px doctor | grep -E "active|sandbox" &&
  px --dry-run install sl | grep "\[dry-run\]"'

echo "distro matrix done"
