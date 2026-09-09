#!/bin/sh
# px installer — the canonical install path:
#   curl -fsSL https://github.com/samuelgirmametaferia/px/releases/latest/download/install.sh | sh
#
# Downloads the latest release binary for the detected arch, verifies it
# runs, installs to ~/.local/bin (creating it, and warning when it's not on
# PATH). Refuses to run as root. No sudo, no system modification.
set -eu

REPO="samuelgirmametaferia/px"
DEST="${PX_INSTALL_DIR:-$HOME/.local/bin}"

[ "$(id -u)" -ne 0 ] || { echo "px installs per-user — run as your normal user, not root" >&2; exit 1; }

# detect arch
case "$(uname -m)" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
case "$(uname -s)" in
  Linux) OS="linux" ;;
  *) echo "unsupported OS: $(uname -s) (px is Linux-first)" >&2; exit 1 ;;
esac

# resolve the latest px VERSION tag — releases/latest is the registry's
# rolling release (registry-latest), not the product
echo "resolving latest version…"
TAGS=$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30")
TAG=$(printf '%s' "$TAGS" | grep -oE '"tag_name": *"v[0-9][^"]*"' | head -1 | grep -oE 'v[0-9][^"]*')
[ -n "$TAG" ] || { echo "could not resolve the latest px release" >&2; exit 1; }
echo "latest release: $TAG"

URL="https://github.com/$REPO/releases/download/$TAG/px"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "↓ downloading px (latest release, $OS/$ARCH)…"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$TMP/px"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMP/px" "$URL"
else
  echo "need curl or wget to download" >&2; exit 1
fi

chmod +x "$TMP/px"
VERSION="$("$TMP/px" --version 2>/dev/null || echo "unknown")"
echo "✓ $VERSION downloaded"

mkdir -p "$DEST"
install -m 755 "$TMP/px" "$DEST/px"
echo "✓ installed → $DEST/px"

case ":$PATH:" in
  *":$DEST:"*) ;;
  *)
    echo ""
    echo "⚠ $DEST is not on your PATH. Add it:"
    echo "    echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.profile  # or your shell's config"
    ;;
esac

echo ""
echo "px is ready. Try:"
echo "  px doctor          — see what px detects on this machine"
echo "  px --tutorial      — learn it in 2 minutes"
echo "  px install anything"
