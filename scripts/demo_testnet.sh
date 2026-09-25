#!/usr/bin/env bash
# Catalysis on Soroban — the covered-vault demo on Stellar testnet.
#
# A vault loses money on a strategy, and the depositor still withdraws in full
# because the insurance pool pays the shortfall at withdrawal. The steps follow
# docs/PLAN.md section 1, and integration-tests/tests/covered_vault_demo.rs
# asserts the same numbers off-chain.
#
#   0. Alice stakes 1,000 XLM into the CoverPool; the curator buys 50 XLM of
#      cover for a 2.5% premium (1.25 XLM), which goes to Alice
#   1-3. A user deposits 100 XLM; the curator allocates 50 to Market A and 50
#      to Market B (mock strategies standing in for two Blend markets)
#   4. Market B fails and loses 20% of its funds (10 XLM). Vault NAV 90
#   5. The user's position: value 90, protected 100, shortfall 10
#   6-7. The user withdraws and receives the full 100 XLM. The pool pays the
#      10 XLM shortfall and slashes Alice: stake 1,000 -> 990, cover 50 -> 40
#   8. Epilogue: Alice collects her premium; the curator closes the cover;
#      after the lock Alice unstakes 990 XLM
#
# Usage:   ./scripts/demo_testnet.sh
#          DEMO_PAUSE=1 ./scripts/demo_testnet.sh   # wait for Enter between steps (for recording)
# Needs:   stellar CLI 28+ (https://developers.stellar.org/docs/tools/cli), jq
#
# Stellar resets testnet periodically; re-run this to redeploy.
set -euo pipefail

NET=testnet
XLM=10000000                 # stroops per XLM
LOCK_LEDGERS=12              # ~1 minute, so the demo can show unstaking
PREMIUM_BPS=250              # 2.5% of the limit
EXPLORER="https://stellar.expert/explorer/testnet"
OUT=deployments/testnet.json
WASM=target/wasm32v1-none/release

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
step() {
  bold "── $* ──"
  if [[ "${DEMO_PAUSE:-0}" == "1" ]]; then read -r -p "press Enter to run" _; fi
}
xlm() { awk -v s="$1" 'BEGIN { printf "%.2f XLM", s / 10000000 }'; }
# invoke <contract> <signer> <fn> [--arg value ...]   sends a transaction
# view   <contract> <fn> [--arg value ...]            simulates only
invoke() { local id=$1 src=$2; shift 2; stellar contract invoke --id "$id" --source "$src" --network $NET -- "$@"; }
view()   { local id=$1; shift; stellar contract invoke --id "$id" --source curator --network $NET --send=no -- "$@"; }
num()    { tr -d '"'; }                                  # i128 results come back quoted
balance() { view "$XLM_SAC" balance --id "$1" | num; }

cd "$(dirname "$0")/.."

# ---------------------------------------------------------------------------
step "0a. Build and fund accounts (curator, alice, user)"
stellar contract build >/dev/null
for who in curator alice user; do
  stellar keys address "$who" >/dev/null 2>&1 || stellar keys generate "$who" --network $NET --fund
done
CURATOR=$(stellar keys address curator)
ALICE=$(stellar keys address alice)
USER=$(stellar keys address user)
XLM_SAC=$(stellar contract id asset --asset native --network $NET)
echo "curator: $CURATOR"
echo "alice:   $ALICE"
echo "user:    $USER"
echo "native XLM contract: $XLM_SAC"

# ---------------------------------------------------------------------------
step "0b. Deploy: CoverPool, CoveredVault, Market A, Market B"
POOL=$(stellar contract deploy --wasm $WASM/cover_pool.wasm --source curator --network $NET \
  -- --admin "$CURATOR" --asset "$XLM_SAC" --lock_ledgers $LOCK_LEDGERS --premium_bps $PREMIUM_BPS)
VAULT=$(stellar contract deploy --wasm $WASM/covered_vault.wasm --source curator --network $NET \
  -- --admin "$CURATOR" --asset "$XLM_SAC" --cover_pool "$POOL")
MARKET_A=$(stellar contract deploy --wasm $WASM/mock_strategy.wasm --source curator --network $NET \
  -- --admin "$CURATOR" --vault "$VAULT" --asset "$XLM_SAC")
MARKET_B=$(stellar contract deploy --wasm $WASM/mock_strategy.wasm --source curator --network $NET \
  -- --admin "$CURATOR" --vault "$VAULT" --asset "$XLM_SAC")
invoke "$POOL"  curator add_vault    --vault    "$VAULT"    >/dev/null
invoke "$VAULT" curator add_strategy --strategy "$MARKET_A" >/dev/null
invoke "$VAULT" curator add_strategy --strategy "$MARKET_B" >/dev/null

mkdir -p deployments
jq -n --arg pool "$POOL" --arg vault "$VAULT" --arg a "$MARKET_A" --arg b "$MARKET_B" \
      --arg xlm "$XLM_SAC" --arg curator "$CURATOR" --arg alice "$ALICE" --arg user "$USER" \
  '{network:"testnet", cover_pool:$pool, covered_vault:$vault,
    mock_strategy_a:$a, mock_strategy_b:$b, native_xlm:$xlm,
    accounts:{curator:$curator, alice:$alice, user:$user}}' > $OUT
cat $OUT
echo "CoverPool:     $EXPLORER/contract/$POOL"
echo "CoveredVault:  $EXPLORER/contract/$VAULT"

# ---------------------------------------------------------------------------
step "0c. Alice stakes 1,000 XLM into the CoverPool"
invoke "$POOL" alice stake --from "$ALICE" --amount $((1000 * XLM)) >/dev/null
echo "pool state:"; view "$POOL" state

# ---------------------------------------------------------------------------
step "0d. The curator buys 50 XLM of cover for the vault (2.5% premium = 1.25 XLM)"
PREMIUM=$(invoke "$VAULT" curator buy_cover --limit $((50 * XLM)) | num)
echo "premium paid:      $(xlm "$PREMIUM")"
echo "vault's cover:";  view "$POOL" cover --vault "$VAULT"
echo "Alice's position (premium is hers):"; view "$POOL" position --staker "$ALICE"

# ---------------------------------------------------------------------------
step "1-3. The user deposits 100 XLM; the curator allocates 50 to Market A and 50 to Market B"
invoke "$VAULT" user    deposit  --from "$USER" --amount $((100 * XLM)) >/dev/null
invoke "$VAULT" curator allocate --strategy "$MARKET_A" --amount $((50 * XLM)) >/dev/null
invoke "$VAULT" curator allocate --strategy "$MARKET_B" --amount $((50 * XLM)) >/dev/null
echo "vault NAV:         $(xlm "$(view "$VAULT" nav | num)")"
echo "Market A:          $(xlm "$(view "$MARKET_A" total_value | num)")"
echo "Market B:          $(xlm "$(view "$MARKET_B" total_value | num)")"

# ---------------------------------------------------------------------------
step "4. Market B fails and loses 20% of its funds (10 XLM)"
LOST=$(invoke "$MARKET_B" curator simulate_loss --bps 2000 | num)
echo "Market B lost:     $(xlm "$LOST")"
echo "Market B now:      $(xlm "$(view "$MARKET_B" total_value | num)")"
echo "vault NAV:         $(xlm "$(view "$VAULT" nav | num)")"

# ---------------------------------------------------------------------------
step "5. The user checks their position: value 90, protected 100, shortfall 10"
POS=$(view "$VAULT" position --depositor "$USER")
echo "$POS"
echo "value:             $(xlm "$(echo "$POS" | jq -r .value)")"
echo "protected:         $(xlm "$(echo "$POS" | jq -r .principal)")"
echo "shortfall:         $(xlm "$(echo "$POS" | jq -r .shortfall)")"

# ---------------------------------------------------------------------------
step "6-7. The user withdraws. The pool pays the shortfall and Alice is slashed"
USER_BEFORE=$(balance "$USER")
SHARES=$(echo "$POS" | jq -r .shares)
PAID=$(invoke "$VAULT" user withdraw --from "$USER" --shares "$SHARES" | num)
USER_AFTER=$(balance "$USER")
echo "user was paid:     $(xlm "$PAID")   (balance +$(xlm $((USER_AFTER - USER_BEFORE))), minus tx fees)"
echo "vault's cover:";   view "$POOL" cover --vault "$VAULT"
echo "pool state:";      view "$POOL" state
echo "Alice's position (1,000 -> 990):"; view "$POOL" position --staker "$ALICE"
echo "Market A:          $(xlm "$(view "$MARKET_A" total_value | num)")"
echo "Market B:          $(xlm "$(view "$MARKET_B" total_value | num)")"

# ---------------------------------------------------------------------------
step "8. Epilogue: Alice collects her premium; the curator closes the cover; Alice unstakes"
PREM=$(invoke "$POOL" alice claim_premium --from "$ALICE" | num)
echo "Alice's premium:   $(xlm "$PREM")"
invoke "$VAULT" curator close_cover >/dev/null
echo "waiting ~70s for the $LOCK_LEDGERS-ledger lock to pass..."
sleep 70
ASHARES=$(view "$POOL" position --staker "$ALICE" | jq -r .shares)
OUT_AMT=$(invoke "$POOL" alice unstake --from "$ALICE" --shares "$ASHARES" | num)
echo "Alice unstaked:    $(xlm "$OUT_AMT")"

bold "Done. Contract IDs are in $OUT"
echo "CoverPool:     $EXPLORER/contract/$POOL"
echo "CoveredVault:  $EXPLORER/contract/$VAULT"
