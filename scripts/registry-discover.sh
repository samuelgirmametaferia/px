#!/usr/bin/env bash
# Discovery: find candidate projects with officially documented installer
# scripts. Sources: GitHub code search for installer patterns in recently
# pushed repos + README install commands. Output: candidate records (JSONL).
# A found install.sh alone is NOT enough — the validator confirms the rest.
set -euo pipefail

# GitHub code search needs auth; the workflow runs with GITHUB_TOKEN.
QUERY_PREFIX='curl -fsSL language:Shell pushed:>2026-08-01'

for q in "curl -fsSL | bash" "curl -fsSL | sh" "wget -qO- | sh"; do
  gh api "search/code?q=$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1]))" "$q $QUERY_PREFIX")" \
    --paginate 2>/dev/null || continue
done | jq -r '.items[]? | .repository.full_name' | sort -u | while read -r repo; do
  # only repos whose README also documents an installer — strong evidence
  readme=$(gh api "repos/$repo/readme" --jq '.content' 2>/dev/null | base64 -d 2>/dev/null) || continue
  if echo "$readme" | grep -qE 'curl.*\|\s*(ba)?sh|wget.*\|\s*(ba)?sh'; then
    installer_url=$(echo "$readme" | grep -oE 'https?://[^ "'"'"')|]*install[^ "'"'"')|]*' | head -1)
    [ -n "$installer_url" ] || continue
    bin=$(echo "$repo" | cut -d/ -f2)
    jq -n --arg cid "github:$repo" --arg repo "$repo" \
        --arg url "$installer_url" --arg bin "$bin" \
        '{canonical_id: $cid, aliases: [$bin],
          repository: ("https://github.com/" + $repo),
          description: "", expected_binaries: [$bin],
          install_methods: [{method: "script", url: $url}],
          identity_confidence: 70, security_state: "unvalidated"}'
  fi
done
