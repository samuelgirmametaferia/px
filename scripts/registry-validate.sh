#!/usr/bin/env bash
# Validate every candidate through the disposable-sandbox validator.
# Only records that pass (receipt issued) move on; failures are dropped
# (not published) — never the other way around.
set -euo pipefail
IN="$1"; OUT="$2"; : > "$OUT"
while read -r rec; do
  echo "$rec" > /tmp/candidate.json
  rm -f /tmp/receipt.json
  if ./registry-infra/validator.sh /tmp/candidate.json /tmp/receipt.json 2>/tmp/validator.err; then
    # attach the receipt hash to the record and promote to validated
    jq --slurpfile r /tmp/receipt.json '. +
      {security_state: "validated",
       identity_confidence: (if .identity_confidence >= 85 then .identity_confidence else 85 end),
       installer_sha256: $r[0].installer_sha256,
       expected_binaries: ($r[0].discovered_binaries | map(sub("^\\./"; ""))),
       validation_receipt_hash: ($r[0] | @base64),
       last_validated_at: $r[0].tested_at}' \
      /tmp/candidate.json >> "$OUT"
  else
    id=$(jq -r .canonical_id /tmp/candidate.json)
    # classify: sudo/tty-dependent installers are legitimately unsuitable for
    # per-user installs (correct drop); anything else is a validator concern
    if grep -q 'no new privileges' /tmp/validator.err; then
      echo "dropped (needs root): $id" >&2
    elif grep -q '/dev/tty' /tmp/validator.err; then
      echo "dropped (interactive): $id" >&2
    else
      echo "validation FAILED, dropping: $id" >&2
      sed 's/^/    /' /tmp/validator.err >&2
    fi
  fi
done < "$IN"
