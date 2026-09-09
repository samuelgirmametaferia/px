#!/usr/bin/env bash
# Merge discovery candidates with unresolved-query-driven candidates,
# drop anything already in the current registry or handled by normal
# package managers, output the validation TODO list.
set -euo pipefail
cat "$1" | sort -u > /tmp/merged.jsonl
[ -f "$2" ] && jq -r '.[].query' "$2" 2>/dev/null >> /tmp/merged.names || true
sort -u /tmp/merged.jsonl
