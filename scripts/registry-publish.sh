#!/usr/bin/env bash
# Publish: upload shards as release assets.
# - dated release (registry-vYYYYMMDD-HHMM): immutable history
# - registry-latest: rolling release, deleted and recreated each publish —
#   px clients point at this tag, so PRODUCT releases can freely be
#   "Latest" without stealing the releases/latest pointer from the registry.
#   Safety is not weakened: root.cbor pins every shard's blake3, so a
#   swapped asset fails verification no matter which release serves it.
set -euo pipefail
DIR="$1"
TAG="registry-v$(date -u +%Y%m%d-%H%M)"

# dated, immutable, for history
gh release create "$TAG" "$DIR"/* --latest=false --title "px registry $TAG" \
  --notes "auto-published by the registry workflow"

# rolling registry-latest for clients
gh release delete registry-latest --yes --cleanup-tag 2>/dev/null || true
gh release create registry-latest "$DIR"/* --latest=false --title "px registry (latest)" \
  --notes "rolling release — px clients resolve from here. contents are
hash-pinned by root.cbor; dated immutable releases keep the history."
echo "published $TAG + registry-latest"
