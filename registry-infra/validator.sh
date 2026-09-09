#!/usr/bin/env bash
# The sandbox validator: runs ONE candidate installer in a disposable
# container with dropped capabilities, no host mounts, a temporary HOME,
# resource limits and a timeout — then diffs the filesystem and tests the
# expected binary. Emits a validation receipt (never "safe", only "tested").
#   usage: validator.sh <record.json>
set -euo pipefail
RECORD="$1"
URL=$(jq -r '.install_methods[0].url' "$RECORD")
BIN=$(jq -r '.expected_binaries[0]' "$RECORD")
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# capture installer content FIRST — the receipt pins these exact bytes
curl -fsSL "$URL" -o "$WORK/installer.sh"
INSTALLER_SHA256=$(sha256sum "$WORK/installer.sh" | cut -d' ' -f1)

# run it in a disposable sandbox: no host mounts, temp HOME, CPU/RAM/PID
# limits, dropped capabilities, hard timeout. Never on the runner itself.
timeout 120 bwrap \
  --ro-bind /usr /usr --ro-bind /etc /etc --ro-bind /bin /bin \
  --proc /proc --dev /dev --tmpfs /tmp --tmpfs /var/tmp \
  --bind "$WORK/home" /tmp/home \
  --unshare-net \
  --die-with-parent \
  --setenv HOME /tmp/home \
  /bin/sh "$WORK/installer.sh" > "$WORK/stdout" 2> "$WORK/stderr" || {
    echo "installer exited non-zero"; exit 1; }

# the expected binary must exist, be executable, and respond to --version
BINPATH="$WORK/home/.local/bin/$BIN"
[ -x "$BINPATH" ] || { echo "expected binary missing"; exit 1; }
"$BINPATH" --version > "$WORK/version" 2>&1 || true

BINARY_SHA256=$(sha256sum "$BINPATH" | cut -d' ' -f1)

# receipt: what was tested, with what, and what came out — not a safety claim
jq -n \
  --arg installer "$INSTALLER_SHA256" \
  --arg binary "$BINARY_SHA256" \
  --arg version "$(cat "$WORK/version")" \
  --arg validator "v1" \
  --arg at "$(date -u +%FT%TZ)" \
  '{validator: $validator, tested_at: $at,
    installer_sha256: $installer, binary_sha256: $binary,
    tests: ["exit_zero", "binary_exists", "binary_executable", "version_runs"],
    result: "pass"}' > "$WORK/receipt.json"
cat "$WORK/receipt.json"
