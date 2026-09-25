# Covered Vault MVP — Build Plan

Handoff for the Claude Code session that builds the pre-grant MVP. Written 25 September 2026. Read `CLAUDE.md` first for rules and conventions; this file is the what and the why.

**Deadline context.** SCF #46 submission closes 8 November 2026. Target: code done, deployed to testnet and demo recorded by **24 October 2026**, leaving two weeks for the application and video.

---

## 1. What we are demoing

One sentence: **a vault that loses money on a strategy, and a depositor who still withdraws in full because the insurance pool pays the shortfall at withdrawal.**

The exact scenario, which is also the integration test and the testnet demo script:

| # | Step | On-chain effect |
|---|---|---|
| 0 | Deploy the CoverPool, then the CoveredVault pointing at it; the pool admin whitelists the vault with `add_vault` | Setup |
| 0 | Alice stakes 1,000 XLM into the CoverPool | Pool capital 1,000 |
| 0 | Vault curator buys 50 XLM of cover for the vault, paying a 2.5% premium (1.25 XLM) | Pool reserves 50 for the vault; premium goes to Alice |
| 1 | The vault accepts XLM deposits | — |
| 2 | The vault has two strategies, Market A and Market B (mocks standing in for two Blend markets) | — |
| 3 | A user deposits 100 XLM; the curator allocates 50 to Market A and 50 to Market B | Vault NAV 100 |
| 4 | Market B fails and loses 20% of its funds (10 XLM) | Vault NAV 90 |
| 5 | The user checks their position: value 90, protected 100, shortfall 10 | View call |
| 6 | The user withdraws. The vault pulls 90 from the strategies, claims the 10 shortfall from the CoverPool, and the pool slashes its stakers by 10 | Alice's stake 1,000 → 990; remaining cover 50 → 40 |
| 7 | The user receives the full 100 XLM | Withdrawal completes |

Plus a quiet-path test: no loss, user withdraws exactly what they put in, no claim, no slashing.

---

## 2. Decisions already made — do not relitigate

- **Payout at withdrawal.** This matches Catalysis on Ethereum. The vault compares what the user is owed (their principal) with what their shares are worth, and claims the difference from the CoverPool in the same transaction. There is no trigger, no oracle and no claims process.
- **Native XLM only.** Native XLM has no issuer, so nobody can freeze or claw back pool capital. No USDC and no RWAs in the MVP.
- **Happy path only.** Build the clean path end to end. Do not build for edge cases or unlikely scenarios. Section 7 lists known trade-offs; document them, don't fix them.
- **Mock strategies now, real Blend in milestone 1.** Mocks are labelled "test and demo only" in code, README and docs.
- **The vault is the covered vault.** No external policy contract, no cover for third-party positions.
- **Fully collateralised pool.** Coverage reserved never exceeds pool capital, and staked capital backing coverage cannot be unstaked. This already holds in the current CoverPool; keep it.
- **soroban-sdk 28.0.0, no other dependencies.** OpenZeppelin's Stellar libraries require soroban-sdk 26 and do not mix.
- **Apache-2.0.**

---

## 3. Current repo state

The repo is at `stellar/catalysis-soroban`, with two commits: 14 tests passing, fmt clean, clippy clean. It was built in an environment that could not build WASM or reach testnet. **Verified 25 September 2026 (Task 0):** fmt, clippy, the 14 tests and `stellar contract build` all pass locally with Rust 1.98.1 and Stellar CLI 28.0.0. Testnet is still unverified; that is Task 6.

| Crate | Now | Action |
|---|---|---|
| `contracts/common` | Cross-contract interfaces + Blend V2 types mirrored from `blend-contracts-v2` | **Keep.** Replace the interfaces (section 4). Keep the Blend types for the milestone-1 Blend strategy |
| `contracts/cover_pool` | Staked XLM pool: shares, lock, premium accumulator, pro-rata slashing, full collateralisation. 11 unit tests | **Keep and modify** (section 4.1) |
| `contracts/policy` | Parametric cover on a Blend position | **Delete** |
| `contracts/blend_adapter` | Read-only `b_rate` reader | **Delete.** Its logic moves into the milestone-1 Blend strategy |
| `contracts/mock_blend_pool` | Mock Blend pool | **Delete** |
| `integration-tests` | Blend-policy scenario | **Rewrite** for the covered vault |
| `scripts/demo_testnet.sh` | Blend-policy demo | **Rewrite** for the covered vault |
| `docs/ARCHITECTURE.md` | Describes the Blend-policy design | **Update** to match (section 8) |
| `data/issuer-flags/` | Evidence for "why native XLM" | Keep unchanged |

---

## 4. Target architecture

```
                         deposit / withdraw
        user ─────────────────────────────────► CoveredVault
                                                 │   │   ▲
                              allocate XLM       │   │   │ claim shortfall
                     ┌───────────────────────────┘   │   │ (at withdrawal)
                     ▼                               │   │
              MockStrategy A     MockStrategy B      │   │
              ("Blend market")   ("Blend market")    │   │
                                                     │   │
                          open_cover / premium       ▼   │
        stakers ──── stake XLM ────────────────► CoverPool
                                                 (slashed pro rata on claim)
```

Three contracts: `covered_vault`, `cover_pool`, `mock_strategy`. Plus `common`.

### 4.1 CoverPool changes

Keep everything about staking as it is: `stake`, `unstake`, `claim_premium`, the lock, the share-value slashing, the premium accumulator, and full collateralisation.

Replace the policy-facing API. Cover is keyed by **vault address** instead of policy id, and supports **many partial claims** against one reservation, because each withdrawal can claim.

```rust
// constructor gains premium_bps (the pool prices its cover)
fn __constructor(env, admin: Address, asset: Address, lock_ledgers: u32, premium_bps: u32);

fn add_vault(env, vault: Address);                          // admin; whitelist of covered vaults

// All three below: vault.require_auth(), vault must be whitelisted.
// The vault is always the direct invoker, so its auth is implicit.
fn open_cover(env, vault: Address, payer: Address, limit: i128) -> i128;
    // reserve `limit` (coverage_written += limit, must stay <= total_capital);
    // charge premium = limit * premium_bps / 10_000, pulled from `payer` into the pool;
    // credit it to stakers via premium_index; returns the premium.
    // One cover per vault; calling again adds to `remaining`.
fn claim(env, vault: Address, amount: i128) -> i128;
    // amount <= cover.remaining;
    // total_capital -= amount; coverage_written -= amount;
    // cover.remaining -= amount; cover.claimed += amount;
    // transfer `amount` to the vault; emit Slashed.
    // This is the slashing moment: every staker's share value falls pro rata.
fn close_cover(env, vault: Address);                        // releases `remaining`

fn cover(env, vault: Address) -> Cover;                     // view: { remaining, claimed }
```

Delete `set_policy_contract`, `reserve`, `release`, `accrue_premium`, `settle_claim` and the `Reservation` type. Update the unit tests: keep the staking ones, and rewrite the policy-facing ones against the new API.

### 4.2 CoveredVault (new)

Accepts native XLM, issues internal shares, allocates to whitelisted strategies, and pays shortfalls from the CoverPool at withdrawal.

```rust
struct Config   { admin: Address, asset: Address, cover_pool: Address }
struct State    { total_shares: i128, idle: i128 }         // idle = XLM held by the vault itself
struct Position { shares: i128, principal: i128 }          // per depositor, persistent
// strategies: Vec<Address> in instance storage

fn __constructor(env, admin: Address, asset: Address, cover_pool: Address);
fn add_strategy(env, strategy: Address);                   // admin
fn buy_cover(env, limit: i128) -> i128;                    // admin pays premium:
    // admin.require_auth(); cover_pool.open_cover(vault, admin, limit)

fn deposit(env, from: Address, amount: i128) -> i128;      // -> shares
    // transfer from -> vault; idle += amount;
    // shares = amount if total_shares == 0 else amount * total_shares / nav (round down);
    // position.shares += shares; position.principal += amount
fn allocate(env, strategy: Address, amount: i128);         // admin; idle -> strategy
    // vault transfers XLM to the strategy address directly (vault is the direct
    // invoker of the token, so no extra auth is needed); idle -= amount

fn withdraw(env, from: Address, shares: i128) -> i128;     // -> amount paid
    // value     = shares * nav / total_shares               (round down)
    // protected = position.principal * shares / position.shares
    // shortfall = max(protected - value, 0)
    // claimable = shortfall - shortfall * DEDUCTIBLE_BPS / 10_000   (DEDUCTIBLE_BPS = 0 in the MVP)
    // pull `value` of liquidity: idle first, then strategy.withdraw(...) in order
    // if claimable > 0: cover_pool.claim(vault, claimable)
    // pay `value + claimable` to `from`
    // position.principal -= protected; position.shares -= shares; total_shares -= shares

fn nav(env) -> i128;                                       // idle + Σ strategy.total_value()
fn position(env, user: Address) -> PositionView;           // { shares, value, principal, shortfall }
fn strategies(env) -> Vec<Address>;
```

Notes for the implementer:

- `position()` shows the demo moment: "value 90, protected 100, shortfall 10".
- Shares are internal and non-transferable in the MVP, which is what makes per-address principal tracking correct.
- A depositor who arrives after a loss deposits at the lower NAV, so their principal equals their value and they have no shortfall. Principal tracking handles this automatically.
- Call order inside `withdraw`: compute `value` from NAV **before** pulling liquidity out of the strategies.
- The deductible exists in the code as a `DEDUCTIBLE_BPS` constant set to zero (founders' decision, 25 September 2026), so the withdrawal maths has the same shape as the Ethereum wrapper's `_capAndApplyDeductible` without changing the demo numbers. Do not make it configurable in the MVP.
- `idle` is stored state, not the vault's live token balance. The shortfall claimed from the pool passes through the vault's balance on its way to the user; do **not** add it to `idle`.
- If a strategy loses everything, NAV can be zero while shares are outstanding and the deposit formula divides by zero. Happy path: do not handle it; it is listed in section 7.

### 4.3 MockStrategy (new) — TEST AND DEMO ONLY

The strategy holds real XLM, so a "loss" really moves tokens out, and the balances visible on the explorer match the story.

```rust
fn __constructor(env, admin: Address, vault: Address, asset: Address);
fn total_value(env) -> i128;                         // token balance of this contract
fn withdraw(env, amount: i128);                      // vault.require_auth(); transfer to vault
fn simulate_loss(env, bps: u32);                     // admin; transfers bps of balance to admin
```

Deposits need no function: the vault transfers XLM straight to the strategy's address. Put the interfaces for strategies and the pool in `common` as `#[contractclient]` traits, as the existing code does. That keeps contract exports from leaking between WASMs.

### 4.4 Why this is safe on Soroban

- **No reentrancy problem.** The vault calls the pool and strategies; they call only the token contract. Nothing calls back into the vault. (Soroban blocks A→B→A calls — the previous design hit exactly that.)
- **Auth is implicit on direct calls.** The vault's own authorization is automatic when it directly invokes the pool or a strategy.
- **The curator signs once for cover.** In `buy_cover`, the admin's signature on the root call covers the nested premium transfer.

---

## 5. Tasks, in order

Each task ends with `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` all passing.

**Task 0 — Install and verify the toolchain (do this first).** ✅ done 25 September 2026
- The founders' Mac has no Rust toolchain at all (checked 25 September 2026): no `rustup`, `cargo` or Stellar CLI. Only `jq` is present.
- Install rustup with the stable toolchain, then `rustup target add wasm32v1-none`
- Install the Stellar CLI (`brew install stellar-cli` or `cargo install --locked stellar-cli`)
- `cargo test --workspace`: the existing 14 tests should pass
- `stellar contract build`: confirm the WASMs build
- Report any problem before changing code

**Task 1 — Delete** ✅ done 25 September 2026 — `policy`, `blend_adapter` and `mock_blend_pool`, and the old Blend-policy integration test. Update the workspace members, the `integration-tests` dev-dependencies, and the interfaces in `common` (keep the Blend types). Update the WASM build step in `.github/workflows/ci.yml`, which names the contract crates explicitly, to `cover-pool`, `covered-vault` and `mock-strategy` (add the last two as their crates land).
- This comes first so that every later task can end with the whole workspace green. Changing the pool API while `policy` still calls the old interface would leave the workspace broken until the rewrite.

**Task 2 — CoverPool API change** (section 4.1). ✅ done 25 September 2026. Update its unit tests.
- Must include a test that three sequential partial claims reduce `remaining` and slash stakers correctly.

**Task 3 — MockStrategy** (section 4.3), with unit tests. ✅ done 25 September 2026.

**Task 4 — CoveredVault** (section 4.2). ✅ done 25 September 2026. Unit tests covering:
- deposit and share minting
- NAV across idle plus two strategies
- withdraw with no loss (no claim)
- withdraw with a loss (claim made, user paid in full)
- a late depositor after a loss has no shortfall
- a partial withdrawal claims a proportional shortfall

**Task 5 — Integration tests** in `integration-tests/tests/`. ✅ done 25 September 2026.
- `covered_vault_demo.rs`: the section 1 table, asserting every number
- a quiet-path test: no loss, exact withdrawal, no slashing

**Task 6 — Rewrite `scripts/demo_testnet.sh`** to run the section 1 scenario on testnet. ✅ done 25 September 2026: verified end to end on testnet three times (the first run found that `close_cover` had to go through the vault; the last run deployed the final WASMs, IDs in `deployments/testnet.json`).
- Keep the `DEMO_PAUSE=1` step mode and the plain-English printouts.
- Print the user's `position` before withdrawal (value 90 / protected 100) and the balances after (user +100, Alice's stake 990).
- Write contract IDs to `deployments/testnet.json`.
- Stellar resets Testnet periodically (roughly quarterly), which wipes deployed contracts. Re-run the script and refresh the IDs in the README shortly before recording the video and before submission.
- Run it for real on testnet and fix it until it passes end to end. The existing script has never been run: check the CLI argument syntax (`--arg_name`, identity names vs addresses, `--send=no` for views) against the installed CLI version.

**Task 7 — Docs.** Update `README.md` and `docs/ARCHITECTURE.md` to match the code (section 8). Put the testnet contract IDs in the README table.

**Task 8 — Ship.**
- The repo lives at `github.com/0xcatalysis/catalysis-soroban`, created 25 September 2026. It is **private until submission** (founders' decision, 25 September 2026). Push; confirm CI is green, including the WASM build step.
- **CI runs locally for now** (founders' decision, 25 September 2026): `./scripts/check.sh` runs fmt, clippy, tests and the WASM build. The GitHub workflow is `workflow_dispatch` only.
- **TODO(founders) before hitting submit:** make the repo public, restore the `push` and `pull_request` triggers in `.github/workflows/ci.yml` and confirm a green run, then re-check that the GitHub URL, the architecture doc link and the README links in the SCF form all open in a logged-out browser. `gh repo edit 0xcatalysis/catalysis-soroban --visibility public --accept-visibility-change-consequences`
- Tag `v0.1.0-mvp`.

---

## 6. Acceptance criteria for the MVP

- `cargo test --workspace` green; CI green on GitHub including the `wasm32v1-none` build
- `./scripts/demo_testnet.sh` runs start to finish on testnet with no manual steps, and the printed numbers match section 1
- Contract IDs in the README and `deployments/testnet.json`, each viewable on stellar.expert
- README and ARCHITECTURE describe exactly what the code does, with mocks and milestone scope clearly labelled

---

## 7. Known trade-offs — document, don't fix

These go in ARCHITECTURE §10 as "known MVP limitations, addressed in milestone 1".

- **First-come compensation if cover is too small.** Payout at withdrawal means that if total shortfall exceeded the vault's remaining cover, early withdrawers would be made whole and later ones not. A withdrawal whose shortfall exceeds the remaining cover fails outright (the pool's claim errors and the whole call reverts); the depositor can withdraw a smaller number of shares whose shortfall fits, or wait for the curator to top up cover. This matches Ethereum, where remaining cover is also first-come at withdrawal. The MVP avoids this by sizing cover above the maximum loss. Milestone 1: record the shortfall when a loss is reported, and share remaining cover pro rata.
- **Stakers can exit ahead of an unclaimed loss.** A loss exists as soon as a strategy fails, but stakers are only slashed when a depositor withdraws. A staker can unstake *excess* capital in between. The capital backing the vault's cover cannot leave, so payouts are always funded; the remaining stakers just carry more of the loss. Milestone 1: vault reports losses to the pool when they happen.
- **No cover expiry or streaming premium.** The curator pays upfront; cover lasts until `close_cover`. On Ethereum, premium streams from depositor principal per second and is split between a platform fee, a pool fee and the restakers (`PremiumManager.distributePremium`). The MVP has no fee split: 100% of the premium goes to stakers.
- **No curator review gate between claims.** On Ethereum, every approved claim sets `claimApprovalRequired` and the curator must call `approveNextClaim` before the next loss-making withdrawal succeeds. The MVP lets the vault claim repeatedly without review.
- **Deductible hardcoded to zero; no per-depositor loss cap.** The Ethereum `CoveredVaultWrapper` caps a claim at `maxCoverableLossBps` of the depositor's basis and subtracts `deductibleBps` before filing. The MVP keeps a `DEDUCTIBLE_BPS` constant in the vault set to zero and has no loss cap, so it pays the full shortfall.
- **Cover cannot be topped up on Ethereum; it can here.** Ethereum policies have a fixed `coverageLimit` and need a new policy for more cover. The MVP's `open_cover` adds to the vault's remaining cover, which is simpler for the demo.
- **A drained vault or pool rejects new money.** If NAV reaches zero with shares outstanding, `deposit` fails with `VaultDrained`; likewise `stake` fails with `PoolDrained` once a pool has been slashed to zero. Neither can be recovered in the MVP; redeploy.
- **Dust-sized operations are rejected, not rounded.** A deposit that would mint zero shares fails (`ZeroShares`), and a withdrawal or unstake that would pay zero fails (`InvalidAmount`). Rounding elsewhere is down, in the contract's favour.
- **Share inflation by donation is only partly mitigated.** Because principal is protected in full, an attacker who donates XLM to a strategy to inflate the share price could make the pool pay a later depositor's rounding loss. The MVP only rejects zero-share mints (the classic first step). Milestone 1: virtual share offset or a minimum first deposit, as in the Ethereum wrapper's decimals offset.
- **The admin triggers losses in the mock strategies.** Demo only.
- **Shares are non-transferable.**
- **The admin can upgrade the contracts.** Multisig and timelock before mainnet.

---

## 8. ARCHITECTURE.md changes (Task 7)

The doc was written for the Blend-policy design. Update:

- **§0 Implementation status** — the covered vault MVP, test count, testnet IDs.
- **§3 System overview** — new diagram and flow (section 4 above).
- **§4 CoverPool** — new API. Keep "why the beneficiary is fixed" only if still accurate: the claimant is now the whitelisted vault itself.
- **§5** — replace "Policy" with **"CoveredVault"**:
  - deposits, strategies, principal tracking
  - payout at withdrawal, and why: it matches the Ethereum product, and needs no trigger and no oracle
  - the worked example from section 1
- **§6 native XLM** — keep as is.
- **§7 loss accounting** — keep. Add that slashing happens at claim time, i.e. at withdrawal.
- **§9 triggers** — the MVP needs none: the shortfall is the vault's own accounting. Blend `b_rate` becomes how the milestone-1 Blend strategy values its position. Oracle triggers are milestone 2 or later.
- **§10** — threat table updated; add the section 7 trade-offs.
- **§11 milestones** — do not change the scope yourself. Leave a clearly marked `TODO(founders)`. The founders are revising milestone scope around the covered vault; a proposal is in section 9 below.

Keep the doc's existing standard: every factual claim sourced, and nothing claimed that the code does not do.

---

## 9. Proposed milestone scope (for the founders, not the code session)

| Milestone | Scope |
|---|---|
| 1 — 31 Jan 2027 | Real Blend V2 strategy on testnet (supply/withdraw via Blend `submit`, value via `b_rate`); loss reporting at strategy level with pro-rata shortfall accounting; cover expiry and streaming premium from vault yield; property tests |
| 2 — 28 Feb 2027 | Multiple Blend markets with curator allocation limits; keeper-based TTL maintenance; USDC vault and pool; STRIDE threat model and monitoring; audit engaged, scope frozen |
| 3 — 31 Mar 2027 | Audit remediation; multisig + timelock; mainnet deployment; first live covered vault with staked capacity; SDK and docs |
