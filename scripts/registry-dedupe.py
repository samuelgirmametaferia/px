#!/usr/bin/env python3
"""Dedupe candidate records by canonical_id (last occurrence wins).
Keeps the candidates branch cumulative across discovery runs."""
import json
import sys

seen = {}
for line in open("candidates.jsonl"):
    line = line.strip()
    if not line:
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        continue
    seen[rec.get("canonical_id", "")] = line

with open("candidates.jsonl", "w") as f:
    for v in seen.values():
        f.write(v + "\n")
