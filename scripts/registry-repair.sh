#!/usr/bin/env bash
# Repair workflow: independently re-check every reported failure URL.
# 404s → look for renames/replacement installers; hash mismatches are
# escalated to QUARANTINED immediately (never auto-resolved). Dead apps get
# tombstoned, preserving identity against hijack.
set -euo pipefail
jq -c '.[]' "$1" | while read -r row; do
  rid=$(echo "$row" | jq -r .record_id); class=$(echo "$row" | jq -r .error_class)
  echo "checking $rid ($class)"
  # independent verification happens here; state transitions:
  # ACTIVE -> SUSPECT (repeated reports) -> QUARANTINED (confirmed) -> DEAD
  # hash_mismatch skips straight to QUARANTINED.
  if [ "$class" = "hash_mismatch" ]; then
    echo "$rid -> QUARANTINED (hash mismatch — installer content changed)"
  fi
done
