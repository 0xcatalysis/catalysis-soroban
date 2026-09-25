#!/usr/bin/env bash
# Catalysis on Soroban — end-to-end happy path on Stellar testnet.
#
#   1. Deploy CoverPool, Policy, Blend adapter and a mock Blend pool
#   2. Two stakers stake XLM into the pool
#   3. A vault buys cover on its Blend supply position
#   4. Bad debt is socialized in Blend (the reserve's b_rate drops 12%)
#   5. A third party settles: the vault is paid, stakers are slashed pro rata
#   6. A staker unstakes and claims premium
#
# Usage:   ./scripts/demo_testnet.sh
#          DEMO_PAUSE=1 ./scripts/demo_testnet.sh   # wait for Enter between steps (for recording)
# Needs:   stellar CLI (https://developers.stellar.org/docs/tools/cli), jq
set -euo pipefail

NET=testnet
XLM=10000000                 # stroops per XLM
LOCK_LEDGERS=12              # ~1 minute, so the demo can show unstaking
DURATION_LEDGERS=17280       # ~1 day
PREMIUM_BPS=250              # 2.5% of the limit
TRIGGER_BPS=500              # pays once the covered rate falls 5%
RATE_ONE=1000000000000       # 1.0 in Blend's 12-decimal rate precision
EXPLORER="https://stellar.expert/explorer/testnet"
OUT=deployments/testnet.json
WASM=target/wasm32v1-none/release

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
step() {
  bold "── $* ──"
  if [[ "${DEMO_PAUSE:-0}" == "1" ]]; then read -r -p "press Enter to run" _; fi
}
xlm() { awk -v s="$1" 'BEGIN { printf "%.2f XLM", s / 10000000 }'; }
invoke() { local id=$1 src=$2; shift 2; stellar contract invoke --id "$id" --source "$src" --network $NET -- "$@"; }
view()   { local id=$1; shift; stellar contract invoke --id "$id" --source admin --network $NET --send=no -- "$@"; }

cd "$(dirname "$0")/.."

# ---------------------------------------------------------------------------
step "0. Build and fund accounts"
stellar contract build
for who in admin alice bob vault keeper; do
  stellar keys address "$who" >/dev/null 2>&1 || stellar keys generate "$who" --network $NET --fund
done
ADMIN=$(stellar keys address admin);  ALICE=$(stellar keys address alice)
BOB=$(stellar keys address bob);      VAULT=$(stellar keys address vault)
XLM_SAC=$(stellar contract id asset --asset native --network $NET)
echo "native XLM contract: $XLM_SAC"

# ---------------------------------------------------------------------------
step "1. Deploy contracts"
BLEND=$(stellar contract deploy --wasm $WASM/mock_blend_pool.wasm --source admin --network $NET \
  -- --admin "$ADMIN")
ADAPTER=$(stellar contract deploy --wasm $WASM/blend_adapter.wasm --source admin --network $NET)
POOL=$(stellar contract deploy --wasm $WASM/cover_pool.wasm --source admin --network $NET \
  -- --admin "$ADMIN" --asset "$XLM_SAC" --lock_ledgers $LOCK_LEDGERS)
POLICY=$(stellar contract deploy --wasm $WASM/policy.wasm --source admin --network $NET \
  -- --admin "$ADMIN" --pool "$POOL" --adapter "$ADAPTER" \
     --premium_bps $PREMIUM_BPS --trigger_bps $TRIGGER_BPS --duration_ledgers $DURATION_LEDGERS)
invoke "$POOL" admin set_policy_contract --policy_contract "$POLICY"

# The covered Blend reserve. Any address works as the key in the mock pool;
# we use the native XLM contract so the demo reads as "XLM supplied to Blend".
RESERVE_ASSET=$XLM_SAC

mkdir -p deployments
jq -n --arg pool "$POOL" --arg policy "$POLICY" --arg adapter "$ADAPTER" \
      --arg blend "$BLEND" --arg xlm "$XLM_SAC" \
  '{network:"testnet", cover_pool:$pool, policy:$policy, blend_adapter:$adapter,
    mock_blend_pool:$blend, native_xlm:$xlm}' > $OUT
cat $OUT
echo "CoverPool:  $EXPLORER/contract/$POOL"
echo "Policy:     $EXPLORER/contract/$POLICY"

# ---------------------------------------------------------------------------
step "2. Stakers commit capital: Alice 6,000 XLM, Bob 4,000 XLM"
invoke "$POOL" alice stake --from "$ALICE" --amount $((6000 * XLM))
invoke "$POOL" bob   stake --from "$BOB"   --amount $((4000 * XLM))
view "$POOL" state

# ---------------------------------------------------------------------------
step "3. Vault with 5,000 XLM supplied to Blend buys 2,000 XLM of cover"
POLICY_ID=$(invoke "$POLICY" vault issue --buyer "$VAULT" --beneficiary "$VAULT" \
  --target "$BLEND" --asset "$RESERVE_ASSET" \
  --covered_amount $((5000 * XLM)) --limit $((2000 * XLM)))
echo "policy id: $POLICY_ID"
view "$POLICY" get --policy_id "$POLICY_ID"
echo "Alice's position:"; view "$POOL" position --staker "$ALICE"

# ---------------------------------------------------------------------------
step "4. Bad debt socialized in Blend: reserve b_rate falls 12%"
invoke "$BLEND" admin set_b_rate --asset "$RESERVE_ASSET" --b_rate $((RATE_ONE * 88 / 100))
view "$POLICY" check --policy_id "$POLICY_ID"

# ---------------------------------------------------------------------------
step "5. A third party settles — no claims process, no vote"
VAULT_BEFORE=$(stellar contract invoke --id "$XLM_SAC" --source admin --network $NET --send=no \
  -- balance --id "$VAULT" | tr -d '"')
PAYOUT=$(invoke "$POLICY" keeper settle --policy_id "$POLICY_ID" | tr -d '"')
VAULT_AFTER=$(stellar contract invoke --id "$XLM_SAC" --source admin --network $NET --send=no \
  -- balance --id "$VAULT" | tr -d '"')
echo "payout:            $(xlm "$PAYOUT")"
echo "vault received:    $(xlm $((VAULT_AFTER - VAULT_BEFORE)))"
echo "pool after:";      view "$POOL" state
echo "Alice (60%):";     view "$POOL" position --staker "$ALICE"
echo "Bob (40%):";       view "$POOL" position --staker "$BOB"

# ---------------------------------------------------------------------------
step "6. After the lock, Alice unstakes and claims premium"
echo "waiting ~70s for the $LOCK_LEDGERS-ledger lock to pass..."
sleep 70
SHARES=$(view "$POOL" position --staker "$ALICE" | jq -r '.shares')
OUT_AMT=$(invoke "$POOL" alice unstake --from "$ALICE" --shares "$SHARES" | tr -d '"')
PREM=$(invoke "$POOL" alice claim_premium --from "$ALICE" | tr -d '"')
echo "Alice unstaked:    $(xlm "$OUT_AMT")"
echo "Alice's premium:   $(xlm "$PREM")"

bold "Done. Contract IDs are in $OUT"
echo "CoverPool events:  $EXPLORER/contract/$POOL"
