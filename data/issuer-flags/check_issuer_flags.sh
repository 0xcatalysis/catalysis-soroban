#!/usr/bin/env bash
# Re-runs the issuer authorization-flag checks behind ARCHITECTURE.md section 6.1
# against the live Stellar ledger and diffs them against the dated snapshot.
#
# Usage:  ./check_issuer_flags.sh [snapshot.json]
# Needs:  curl, jq
set -euo pipefail

SNAPSHOT="${1:-$(dirname "$0")/snapshot-2026-09-25.json}"
HORIZON="${HORIZON:-https://horizon.stellar.org}"

command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }

printf '%-28s %-9s %-9s %-9s %-9s %-26s %s\n' \
  ISSUER REVOCABLE CLAWBACK IMMUTABLE REQUIRED HOME_DOMAIN CHANGED
drift=0

while IFS=$'\t' read -r label account; do
  live="$(curl -fsS "$HORIZON/accounts/$account")"
  read -r rev claw imm req dom < <(jq -r \
    '[.flags.auth_revocable, .flags.auth_clawback_enabled,
      .flags.auth_immutable, .flags.auth_required, (.home_domain // "-")] | @tsv' <<<"$live")

  snap="$(jq -c --arg a "$account" '.issuers[] | select(.account_id==$a) | .flags' "$SNAPSHOT")"
  now="$(jq -c '.flags | {auth_required, auth_revocable, auth_immutable, auth_clawback_enabled}' <<<"$live")"
  was="$(jq -c '{auth_required, auth_revocable, auth_immutable, auth_clawback_enabled}' <<<"$snap")"

  changed="no"
  if [[ "$now" != "$was" ]]; then changed="YES"; drift=1; fi
  printf '%-28s %-9s %-9s %-9s %-9s %-26s %s\n' "$label" "$rev" "$claw" "$imm" "$req" "$dom" "$changed"
done < <(jq -r '.issuers[] | [.label, .account_id] | @tsv' "$SNAPSHOT")

if [[ $drift -eq 1 ]]; then
  echo; echo "Flag state has changed since the snapshot. Section 6.1 needs updating." >&2
  exit 2
fi
echo; echo "All issuer flags match the snapshot."
