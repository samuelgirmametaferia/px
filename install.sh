#!/bin/sh
# Per-user installer. Each published copy defaults to its own product version;
# registry releases must never influence which executable gets installed.
set -eu

REPO="samuelgirmametaferia/px"
DEFAULT_VERSION="v3.1.0"
TAG="${PX_VERSION:-$DEFAULT_VERSION}"
DEST="${PX_INSTALL_DIR:-$HOME/.local/bin}"

fail() { echo "px: $*" >&2; exit 1; }
[ "$(id -u)" -ne 0 ] || fail "run as your normal user, not root"
case "$TAG" in
  v[0-9]*) ;;
  *) fail "PX_VERSION must be a release tag such as v3.1.0" ;;
esac
case "$TAG" in *[!a-zA-Z0-9._-]*) fail "invalid release tag: $TAG" ;; esac

case "$(uname -s)" in
  Linux) OS="linux" ;;
  *) fail "prebuilt binaries support Linux; build from source for other systems" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac

if command -v curl >/dev/null 2>&1; then
  download() { curl --connect-timeout 15 --max-time 180 -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget --timeout=30 --tries=2 -qO "$2" "$1"; }
else
  fail "install curl or wget first"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' 0
trap 'exit 130' INT
trap 'exit 143' TERM

BASE="https://github.com/$REPO/releases/download/$TAG"
ASSET="px-$OS-$ARCH"
# v3.1.0 and older shipped only an x86_64 executable named px. Never
# hand that binary to ARM users; newer releases use explicit asset names.
echo "↓ downloading px $TAG ($OS/$ARCH)…"
if ! download "$BASE/$ASSET" "$TMP/px"; then
  if [ "$ARCH" = x86_64 ]; then
    download "$BASE/px" "$TMP/px" || fail "no binary available for $TAG ($OS/$ARCH)"
  else
    fail "no ARM binary available for $TAG; build with cargo install --path . --bin px"
  fi
fi

chmod +x "$TMP/px"
VERSION="$("$TMP/px" --version 2>/dev/null)" || fail "downloaded binary cannot run on this system; existing installation unchanged"
[ "$VERSION" = "px ${TAG#v}" ] || fail "downloaded binary reports '$VERSION', expected 'px ${TAG#v}'"

mkdir -p "$DEST"
# Stage beside the destination so replacement is atomic and works even
# when the installed executable is currently running.
STAGED="$(mktemp "$DEST/.px-install.XXXXXX")"
trap 'rm -rf "$TMP"; rm -f "$STAGED"' 0
install -m 755 "$TMP/px" "$STAGED"
mv -f "$STAGED" "$DEST/px"
echo "✓ installed $VERSION → $DEST/px"

case ":${PATH:-}:" in
  *":$DEST:"*) ;;
  *)
    echo ""
    echo "Add this directory to PATH in your shell configuration:"
    echo "  $DEST"
    ;;
esac

echo ""
echo "Try: px doctor, px --tutorial, or px --dry-run install ripgrep"
