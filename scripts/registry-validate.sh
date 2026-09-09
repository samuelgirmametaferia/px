#!/usr/bin/env bash
# Validate every candidate through the disposable-sandbox validator.
# Only records that pass (receipt issued) move on; failures are dropped
# (not published) — never the other way around.
set -euo pipefail
IN="$1"; OUT="$2"; : > "$OUT"
while read -r rec; do
  echo "$rec" > /tmp/candidate.json
  if ./registry-infra/validator.sh /tmp/candidate.json > /tmp/receipt.json 2>/dev/null; then
    # attach the receipt hash to the record and promote to validated
    jq --slurpfile r /tmp/receipt.json '. +
      {security_state: "validated",
       identity_confidence: (if .identity_confidence >= 85 then .identity_confidence else 85 end),
       installer_sha256: $r[0].installer_sha256,
       validation_receipt_hash: ($r[0] | @base64),
       last_validated_at: $r[0].tested_at}' \
      /tmp/candidate.json >> "$OUT"
  else
    echo "validation FAILED, dropping: $(jq -r .canonical_id /tmp/candidate.json)" >&2
  fi
done < "$IN"
