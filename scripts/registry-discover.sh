#!/usr/bin/env bash
# Discovery: find candidate projects with officially documented installer
# scripts. Sources: GitHub code search for installer patterns in recently
# pushed repos + README install commands. Output: candidate records (JSONL).
# A found install.sh alone is NOT enough — the validator confirms the rest.
set -euo pipefail

# GitHub code search needs auth; the workflow runs with GITHUB_TOKEN.
QUERY_PREFIX='curl -fsSL language:Shell pushed:>2026-08-01'

# repo search (NOT code search — the runner's app token gets silently
# empty results from search/code, and 429s when pushed). search/repositories
# works with GITHUB_TOKEN and supports in:readme. ONE rotated query per run.
QUERIES=('"curl -fsSL | bash" in:readme' '"curl -fsSL | sh" in:readme' '"wget -qO- | sh" in:readme')
q="${QUERIES[$(( $(date -u +%u) % ${#QUERIES[@]} ))]}"
encoded=$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1]))" "$q pushed:>2026-08-01")
for attempt in 1 2; do
  if gh api "search/repositories?q=$encoded&sort=updated&per_page=30"; then
    break
  fi
  echo "search failed (attempt $attempt), backing off 30s" >&2
  sleep 30
done | jq -r '.items[]? | .full_name' | sort -u | while read -r repo; do
  # only repos whose README also documents an installer — strong evidence
  readme=$(gh api "repos/$repo/readme" --jq '.content' 2>/dev/null | base64 -d 2>/dev/null) || {
    echo "no readme: $repo" >&2
    continue
  }
  if echo "$readme" | grep -qE 'curl.*\|\s*(ba)?sh|wget.*\|\s*(ba)?sh'; then
    installer_url=$(echo "$readme" | grep -oE 'https?://[^ "'"'"')|]*install[^ "'"'"')|]*' | head -1)
    [ -n "$installer_url" ] || { echo "no installer url in readme: $repo" >&2; continue; }
    echo "candidate: $repo" >&2
    bin=$(echo "$repo" | cut -d/ -f2)
    jq -nc --arg cid "github:$repo" --arg repo "$repo" \
        --arg url "$installer_url" --arg bin "$bin" \
        '{canonical_id: $cid, aliases: [$bin],
          repository: ("https://github.com/" + $repo),
          description: "", expected_binaries: [$bin],
          install_methods: [{method: "script", url: $url}],
          identity_confidence: 70, security_state: "unvalidated"}'
  fi
done
