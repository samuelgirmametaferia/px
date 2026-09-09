#!/usr/bin/env bash
# Publish: upload shards as immutable release assets, atomically replace
# the root. Old clients keep using the previous snapshot.
set -euo pipefail
DIR="$1"
TAG="registry-v$(date -u +%Y%m%d-%H%M)"
# upload every shard + root as assets on a fresh immutable release
gh release create "$TAG" "$DIR"/* --title "px registry $TAG" \
  --notes "auto-published by the registry workflow" --latest
echo "published $TAG"
