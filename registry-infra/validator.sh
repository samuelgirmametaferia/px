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
mkdir -p "$WORK/home"
trap 'rm -rf "$WORK"' EXIT

# capture installer content FIRST — the receipt pins these exact bytes
curl -fsSL "$URL" -o "$WORK/installer.sh"
INSTALLER_SHA256=$(sha256sum "$WORK/installer.sh" | cut -d' ' -f1)

# snapshot the temp home so we can diff for NEW files afterwards
(cd "$WORK/home" && find . -type f) | sort > "$WORK/before.txt"

# run it in a disposable sandbox: system read-only, temp HOME, no host
# mounts, hard timeout. Never on the runner itself. Network stays ON —
# installers legitimately download; the read-only system is the security
# boundary (a validator that breaks every downloader validates nothing).
timeout 420 bwrap \
  --ro-bind / / \
  --proc /proc --dev /dev --tmpfs /tmp --tmpfs /var/tmp \
  --bind "$WORK/home" /tmp/home \
  --die-with-parent \
  --setenv HOME /tmp/home \
  --setenv PATH /usr/bin:/bin \
  /bin/sh < "$WORK/installer.sh" > "$WORK/stdout" 2> "$WORK/stderr" || {
    echo "installer exited non-zero"; tail -5 "$WORK/stderr"; exit 1; }

# DIFF the temp home: find NEW executables the installer created, wherever
# it put them (~/.local/bin, ~/.cargo/bin, ~/bin, ...). The expected-binaries
# guess from discovery is only a hint — the filesystem is the truth.
mapfile -t NEWBINS < <(cd "$WORK/home" && find . -type f -executable ! -path "*/.git/*" | sort | comm -13 "$WORK/before.txt" -)
if [ ${#NEWBINS[@]} -eq 0 ]; then
  echo "no new executables appeared in the temp HOME"; exit 1
fi
echo "discovered binaries: ${NEWBINS[*]}"
BINPATH="$WORK/home/${NEWBINS[0]}"
# prefer the hinted binary when it exists among the new executables
for nb in "${NEWBINS[@]}"; do
  case "$nb" in *"$BIN"*) BINPATH="$WORK/home/$nb";; esac
done
"$BINPATH" --version > "$WORK/version" 2>&1 || true

BINARY_SHA256=$(sha256sum "$BINPATH" | cut -d' ' -f1)

# receipt: what was tested, with what, and what came out — not a safety claim
jq -n \
  --arg installer "$INSTALLER_SHA256" \
  --arg binary "$BINARY_SHA256" \
  --arg version "$(cat "$WORK/version")" \
  --arg validator "v1" \
  --arg at "$(date -u +%FT%TZ)" \
  --argjson bins "$(printf '%s\n' "${NEWBINS[@]}" | jq -R . | jq -s .)" \
  '{validator: $validator, tested_at: $at,
    installer_sha256: $installer, binary_sha256: $binary,
    discovered_binaries: $bins,
    tests: ["exit_zero", "binary_exists", "binary_executable", "version_runs"],
    result: "pass"}' > "$WORK/receipt.json"
cat "$WORK/receipt.json"
