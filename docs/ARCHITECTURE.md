# Catalysis on Soroban — Technical Architecture

**Covered vaults for Stellar: staked XLM insurance that pays depositor shortfalls at withdrawal**

Version 1.3 · 25 September 2026 · Prepared for Stellar Community Fund #46, Build Award (Open Track)
Authors: Abhishek Kumar, Dhruv Bodani · Catalyst Labs LLC FZ

---

## 0. Scope

This specifies the on-ledger design of a coverage protocol for Stellar: the contracts, their storage, their interfaces, the accounting method, how a shortfall is determined, and the integration surfaces against Blend V2 and DeFindex. It is the design the SCF #46 build executes against.

It does not restate our Ethereum implementation. The Soroban system differs from it in its capital model, its loss accounting and its trigger construction, and section 2 gives the specific properties of Stellar that force each difference.

Every figure carries a source. Where sources disagree the range is given rather than a point estimate, and where a claim could not be verified from a primary source it is marked and the design does not rest on it.

**Implementation status.** The covered-vault happy path in sections 3 to 7 is implemented, tested and deployed to testnet at https://github.com/0xcatalysis/catalysis-soroban: CoverPool, CoveredVault, a mock strategy standing in for a Blend market (test and demo only), 83 passing tests including the end-to-end demo scenario, and a script that has run that scenario on testnet. Contract IDs are in the README and in `deployments/testnet.json`. Anything scheduled for a milestone is marked as such where it appears.

**Reading order for a reviewer with limited time:** section 2 (the constraints), section 4 (the staked pool and its slashing guarantee), section 5 (the covered vault and payout at withdrawal), section 6 (why the pool holds native XLM, which has no Ethereum analogue).

---

## 1. The gap this closes

Stellar has roughly $3.4B to $4.0B of tokenized real-world assets issued on-ledger, depending on methodology. A Dune dashboard maintained by Stellar (dune.com/stellar/rwas) reported $3.996B as of 29 August 2026, up from $868.8M at end-2025; rwa.xyz reports approximately $3.45B today, of which $3.37B is distributed. Spiko is the largest issuer on both counts at $1.55B to $1.7B.

Against that, Stellar's entire DeFi TVL is about $260M, and Blend — the chain's primary lending market — holds $164.4M with $43.7M in active loans. The overwhelming majority of tokenized value on Stellar does not work as collateral anywhere. The constraint is not issuance and it is not contract quality. It is that nobody owns credit-risk curation, nobody is paid for it, and when curation fails there is no capital positioned to absorb the loss.

### 1.1 What happened on 22 February 2026

At 00:25 UTC an attacker manipulated the price of USTRY — an Etherfuse tokenized US Treasury instrument — in a Blend V2 pool operated by the YieldBlox DAO, and borrowed 1,000,196.70 USDC and 61,249,278.31 XLM against collateral worth a small fraction of its marked value.

The mechanism matters, because the design below is shaped by each step of it:

- The pool priced USTRY through a Reflector feed sourced from the Stellar DEX. The canonical mainnet Stellar DEX feed is `CALI2BYU2JE6WVRUFYTS6MSBNEHGJ35P4AVCZYF3B6QOE3QKOB2PLE6M`; we have not confirmed from a primary source that this pool read that contract directly rather than a pool-specific deployment, and section 9.2 explains why that distinction may matter.
- The USTRY/USDC market on that venue was extremely thin — rekt.news reports under $1 in hourly volume and fewer than five tokens on the ask side.
- The attacker moved the price roughly a hundredfold, from about $1.05 (Halborn) or $1.06 (rekt.news) to about $106.7, with a trade small enough to be irrelevant in any deeper market.
- The oracle captured the manipulated price and the borrow landed minutes later.

Dollar totals differ across sources because the XLM leg is marked at different prices: rekt.news and DefiLlama's hack database report $10.97M, Bankless $10.8M, Halborn $10.2M, BlockSec "exceeding $10 million." Token amounts are consistent everywhere. **Roughly $10.2M to $11.0M is the honest range.**

Three properties matter more than the number.

**The contracts were not broken.** Blend V2's core had been through Certora formal verification. The verified properties were not the ones that failed: the loss came from pool configuration — the choice of oracle and the parameters set around it. Script3, Blend's developer, characterised it as a pool-operator configuration issue rather than a Blend V2 bug, and that is the correct characterisation. It is also precisely the risk surface that no mechanism on Stellar prices.

**Resolution was discretionary.** Script3 stated that all USDC, XLM and EURC depositors in the affected pool would be fully compensated. Recovery also depended on Stellar Tier-1 validators freezing roughly 48 million XLM, about 80% of the XLM taken, and a 10% white-hat bounty with a 72-hour deadline was offered and ignored. Every one of those is a social intervention: unrepeatable, unguaranteed, and unattractive to the institutional allocators the chain is courting. Public reporting does not state whether the pool's backstop was fully drawn down or merely insufficient, and does not identify the source of the compensation funds; we assert neither.

**Under Blend's documented design, residual bad debt is socialized among that pool's lenders.** Bad debt transfers to the backstop, a bad-debt auction sells backstop holdings, and per the Blend whitepaper "any remaining bad debt is socialized among lenders if backstop module deposits are insufficient." Compensation offset that outcome here. The mechanism remains.

### 1.2 Why the existing buffer cannot be the answer

Blend's backstop is the only bad-debt absorption mechanism of consequence on Stellar. It is denominated in **BLND:USDC 80:20 liquidity pool shares**, per Blend's documentation. The contract's backstop threshold test computes `(bal_blnd⁴ × bal_usdc) / PC⁵` against `1e25` with `PC = 100_000`; the 4:1 exponent ratio mirrors those Comet pool weights.

This is wrong-way risk, and it is not hypothetical:

- **The collateral is mostly the governance token of the protocol being backstopped.** BLND trades at $0.0053, down about 95.6% from its November 2024 high of $0.12 and down 37% over thirty days. Capacity denominated in that asset degrades exactly as confidence in the protocol degrades.
- **The collateral lives in an AMM pool whose bugs are the backstop's bugs.** In late August 2026 a vulnerability in same-asset swaps in the Comet AMM — the BLND-USDC pool collateralising the backstop — was reported to have caused a loss of roughly $717,000, and Blend's operators were reported to have paused the backstop. That figure and the pause come from a single originating outlet (DailyCoin, 28 August 2026, syndicated onward), unconfirmed by any security firm or by Script3, so we hold them loosely. The bug class is independently corroborated in source: Comet v1's error enum carries a dedicated `ErrSameToken` variant alongside overflow and minimum-balance guards. We could not locate a release note describing the fix and do not assert one.
- **It is not quickly exitable, and its exit is observable.** Backstop withdrawals run through a 17-day queue, and `q4w_pct` on the backstop's pool data exposes how much is already queued.

A first-loss buffer bounded in practice by the depth of its own collateral, held inside a contract that is itself a solvency surface, is not a risk-transfer system. It is a deductible.

*(Contemporaneous reporting also claimed Stellar TVL fell 60% in a single day to $98M and Blend's TVL fell "to nearly zero." DefiLlama's own historical series contradicts both — Stellar chain TVL was $244.6M on 27 August against a 22 August peak of $264.9M, and Blend was $152.8M. We do not repeat those figures.)*

### 1.3 What Catalysis adds

A **second-loss layer**: exogenous underwriting capital committed to a named vault under contract, with the payout computed on-ledger from the vault's own accounting and paid inside the depositor's withdrawal. Capital that is not the covered protocol's own token. No claims committee, no governance vote, no discretionary compensation, no validator coordination.

We have built this once, on Ethereum, audited and live. That is the reason to fund the Stellar version and not the design for it — what follows is a different system in the ways Stellar requires.

---

## 2. The constraints that shape this design

Each item below is a property of Stellar or Soroban that changed a decision. Removing any one of them breaks the design.

**2.1 Issuer authorization flags reach inside contract balances.** An asset issuer's authorization state applies to a balance held by a Soroban contract, not only to classic trustlines. The Stellar Asset Contract documentation is explicit: deauthorization "is true for both the trustline balances of `Address::Account` and contract balances of `Address:Contract`," and "a balance can only be clawed back if the issuer account had the `AUTH_CLAWBACK_ENABLED_FLAG` set when the balance was created." A contract address is not a shelter. For a coverage protocol this is existential — capital an issuer can freeze is not capacity — and it has no Ethereum analogue. Section 6 is the response.

**2.2 There is no restaking, no shared security, and no slashable external capital.** On Ethereum our capacity comes from an existing market: restakers hold slashable EigenLayer and Symbiotic delegations, and someone else built the slashing rail. Stellar has no equivalent. Capacity must be raised, committed, priced and drawn natively. This is the largest piece of new engineering in this build.

**2.3 Footprint limits bound the shape of loss accounting, not just its cost.** Soroban resource limits are on-ledger network settings set by validator vote, not constants. SLP-0004 (Final, 12 January 2026) raised per-transaction write entries from 50 to 200, per-ledger writes from 500 to 1000, and footprint entries from 100 to 200; SLP-0005 (Final, 18 February 2026) raised footprint entries to 400. Protocol 23 (CAP-0066) also changed *what* is metered: `txMaxDiskReadEntries` counts disk reads only, in-memory reads carry no entry- or byte-based fee and no byte limit but remain bounded by an entry-count limit, and `txMaxFootprintEntries` caps read-only and read-write entries combined. The consequence is unchanged by the increase: **settlement must never iterate over staker positions.** A pool with a thousand stakers cannot be settled by touching a thousand entries, at any limit. Section 7 makes settlement constant in staker count.

**2.4 There is no multicall.** stellar-core enforces one operation per Soroban transaction and rejects mixing Soroban with non-Soroban operations. Batching happens *inside* a single invocation, in a loop bounded by the live footprint limit — so batch sizes are contract parameters read from storage, never compile-time constants.

**2.5 Persistent state archives, and the fix is bundled, not separate.** Every staker position, every depositor position and every vault's cover is a persistent ledger entry with its own TTL; minimum and maximum TTLs are network parameters. Two properties we rely on: TTL extension has no access control — "any user may invoke `ExtendFootprintTTLOp` on any LedgerEntry" — so a keeper can maintain positions without a signature from their owner; and from protocol 23 archived persistent entries in a transaction's restore list are restored automatically when the invocation executes, so a claim against an archived entry needs no separate `RestoreFootprintOp` (which must be the only operation in its transaction and would therefore be unusable mid-settlement). The docs also warn that "TTL extensions should never be relied on for functionality or safety." Section 8 takes that seriously.

**2.6 Reflector gives prices, not price analytics, and its resolution is coarse.** Reflector's public Pulse feeds default to a 5-minute resolution with a 24-hour default history retention period, and expose SEP-40's `price`, `prices` and `lastprice` — **no TWAP function**. Time weighting is computed consumer-side from `prices(asset, records)`, and retention bounds any window at 24 hours. Reflector's Beam product updates faster but charges an XRF fee per read and requires the caller to authorize a burn, putting a token dependency and a sub-invocation in the read path; we design against Pulse. Both `resolution()` and `history_retention_period()` are contract-readable and we read them rather than assuming the defaults.

**2.7 A Soroban contract cannot read the classic DEX orderbook.** This is the constraint an Ethereum-native design would get wrong. The venue whose thinness enabled the February manipulation is not introspectable from inside a contract: there is no host function returning SDEX order book depth. What *is* readable on-ledger is AMM pool reserves and a second, independent Reflector feed — the external CEX/DEX feed (`CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN`) alongside the Stellar DEX feed. Cross-feed divergence is the only depth proxy Stellar makes available to a contract. Section 9.2 specifies it, including the case — common for RWAs — where the second feed does not quote the asset at all.

**2.8 A Soroban contract cannot read another contract's storage, or a classic account's flags.** `e.storage()` is namespaced to the invoking contract; there is no arbitrary-ledger-entry host function, and the host exposes no account-flag accessor. Everything we learn about Blend, DeFindex or an issuer comes through a public function that contract chooses to expose, or through an attestation written on-ledger by a keeper. Sections 6.3 and 9.1 are both shaped by this, and both say so where a naive design would assume otherwise.

**2.9 Cross-contract reentrancy is blocked by the host.** `rs-soroban-env` defaults every external contract call to `ContractReentryMode::Prohibited`, and detection scans the whole context stack, so indirect reentry (A→B→A) is caught. This removes a class of attack and also a pattern: no callback-driven settlement, no reentrant vault hooks. Settlement resolves state fully before any external transfer.

**2.10 Upgrades replace Wasm and do not re-run constructors.** `update_current_contract` preserves address and storage, and CAP-0058 states that on code update a contract "is not considered created and thus constructor won't be called," so a constructor in the current Wasm carries no guarantee of having run. The MVP has an admin-only `upgrade` on each contract and no state migration; an explicit, idempotent, version-gated `migrate()` arrives with the multisig in milestone 3. There is no delegatecall and contract size is capped, so the system is several contracts rather than one.

**2.11 Network liquidity is thin and concentrated in a single venue.** Aquarius carries about 98.7% of DefiLlama-tracked Stellar DEX volume — $5.28M of $5.35M over 24 hours, $69.24M of $70.87M over 30 days, as of 25 September 2026. **No part of this system may assume it can liquidate collateral on-ledger at size.** Claims settle in the asset the pool already holds. We never sell collateral into a Stellar venue to fund a payout.

---

## 3. System overview

Three contracts. Two are the product: the CoverPool and the CoveredVault. The third, MockStrategy, stands in for a Blend market and exists only for tests and the testnet demo; milestone 1 replaces it with a strategy that supplies to a real Blend V2 pool.

```
┌───────────────────────────────────────────────────────────────────────┐
│ STELLAR / SOROBAN                                                     │
│                                                                       │
│                     deposit / withdraw                                │
│   Depositor ──────────────────────────────►┌──────────────────────┐   │
│   (paid in full)                           │ (2) CoveredVault     │   │
│                                            │  shares · principal  │   │
│        ┌─────────────── allocate ──────────│  idle · strategies   │   │
│        │                                   └───┬──────────────▲───┘   │
│        ▼                                       │ buy_cover    │ claim │
│  ┌──────────────┐  ┌──────────────┐            │ (premium)    │ short-│
│  │ Strategy A   │  │ Strategy B   │            ▼              │ fall  │
│  │ (Blend mkt)  │  │ (Blend mkt)  │       ┌──────────────────────┐    │
│  └──────────────┘  └──────────────┘       │ (1) CoverPool        │    │
│   test and demo: MockStrategy             │  staked XLM · shares │    │
│   milestone 1:  Blend V2 strategy         │  premium index       │    │
│                                           └──────────▲───────────┘    │
│                                                      │ stake/unstake  │
│                                           ┌──────────┴───────────┐    │
│                                           │ Stakers (native XLM) │    │
│                                           └──────────────────────┘    │
└───────────────────────────────────────────────────────────────────────┘
```

Capital custody, vault accounting and settlement all execute on Stellar; nothing in the payout path leaves the ledger, and nothing in it reads a price.

**The flow.** Stakers stake XLM into a CoverPool and receive shares under a lock period. The vault's curator buys cover for the vault: the pool reserves the limit and charges a premium, which goes to the stakers. Depositors put XLM into the vault and receive internal shares; the vault records each depositor's principal. The curator allocates idle XLM to whitelisted strategies. If a strategy loses money, the vault's net asset value falls and every depositor's shares are worth less than their principal. When a depositor withdraws, the vault values their shares, claims the difference between principal and value from the CoverPool in the same transaction, and pays the depositor in full. The claim lowers the pool's capital, so every staker is slashed pro rata through the share value. There is no trigger, no oracle and no claims process: the shortfall is the vault's own accounting.

**This is the Ethereum product, not a new one.** On Ethereum, Catalysis wraps a Morpho vault and files its claim inside the depositor's withdrawal. Here the vault and the wrapper are one contract, because Stellar has no curated XLM vault to wrap; section 5 says what carries over and what is deliberately simpler.

**Positions are entries, not contract instances.** Each staker's and each depositor's position is a persistent ledger entry keyed by address in the pool or the vault. A factory-per-position design would put a contract instance and its own rent under every position, and Soroban's instance storage shares the contract's TTL, so a thousand positions would mean a thousand instances to keep alive.

---

## 4. Contract 1 — CoverPool

A staked capital pool that insures covered vaults. This is the Stellar-native replacement for the EigenLayer and Symbiotic capacity our Ethereum system draws on (2.2): not restaking, but a plain capital pool with slashing on claim, which is what the coverage layer actually needs. Source: [`contracts/cover_pool`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/cover_pool).

### 4.1 Storage

```rust
#[contracttype]
pub struct PoolConfig {
    pub admin: Address,
    pub asset: Address,        // Stellar Asset Contract of the staked asset: native XLM
    pub lock_ledgers: u32,     // minimum stake duration
    pub premium_bps: u32,      // premium charged on open_cover, as bps of the limit
}

#[contracttype]
pub struct PoolState {
    pub total_capital: i128,   // asset units backing all shares
    pub total_shares: i128,
    pub coverage_written: i128,// sum of every vault's remaining cover
    pub premium_index: i128,   // cumulative premium per share, 1e18 fixed point
    pub premiums_paid: i128,
    pub claims_paid: i128,
}

#[contracttype]
pub struct Position {          // one persistent entry per staker
    pub shares: i128,
    pub premium_index_entry: i128,
    pub premium_owed: i128,
    pub locked_until: u32,
}

#[contracttype]
pub struct Cover {             // one persistent entry per covered vault
    pub remaining: i128,       // what can still be claimed
    pub claimed: i128,         // running total already paid out
}
```

The whitelist of covered vaults is a set of instance-storage keys: it is small and admin-managed.

### 4.2 Interface

```rust
// stakers — each requires the staker's own auth
fn stake(env, from: Address, amount: i128) -> i128;          // -> shares
fn unstake(env, from: Address, shares: i128) -> i128;        // -> amount
fn claim_premium(env, from: Address) -> i128;

// admin
fn add_vault(env, vault: Address);                            // whitelist a covered vault

// vault-facing — each requires the vault's own auth and the vault to be whitelisted
fn open_cover(env, vault: Address, payer: Address, limit: i128) -> i128;  // -> premium
fn claim(env, vault: Address, amount: i128) -> i128;          // partial, repeatable
fn close_cover(env, vault: Address);                          // releases what remains

// views and operations
fn available_capacity(env) -> i128;
fn cover(env, vault: Address) -> Cover;
fn is_vault(env, vault: Address) -> bool;
fn state(env) -> PoolState;
fn position(env, staker: Address) -> PositionView;   // shares, value, premium, lock
fn share_value(env) -> i128;
fn bump_position(env, staker: Address);             // permissionless TTL extension
fn upgrade(env, wasm_hash: BytesN<32>);              // admin
```

**The claimant is the vault itself, and there is no recipient parameter.** `claim` pays the vault that calls it, against that vault's own cover. On Soroban a contract's authorization is implicit when it is the direct invoker, so the pool's `vault.require_auth()` passes only when the vault contract itself makes the call; a plain account cannot claim on a vault's behalf. Nothing in the interface can direct a payout anywhere except to the whitelisted vault whose cover it draws on, and the pool caps every claim at that vault's remaining cover. The pool never calls back into the vault (2.9): the vault calls the pool, the pool calls only the token, and the vault pays its depositor afterwards.

**Authorization.** The three vault-facing functions require the vault's auth and reject vaults the admin has not whitelisted. Staker functions require the staker's own auth. `add_vault` and `upgrade` require the admin. `bump_position` and the views are permissionless because they cannot move value. The premium payer authorizes the premium transfer through the token: in practice the vault's curator signs `buy_cover` on the vault, and that signature covers the nested transfer.

### 4.3 Design decisions

**Custody-lock, never allowance.** `stake` moves the asset into the pool contract's own balance in the staker's signed transaction. From that ledger the pool owns the funds and needs no staker authorization to pay a claim. The alternative fails on its own terms: SEP-41 states that "an allowance is a spending limit, not a reservation. It does not lock any of `from`'s balance and may exceed it," and a staker could revoke it the moment a loss looked likely.

**Segregated pools, not one mutual.** A pool insures the vaults whitelisted on it, so a staker chooses the risk they hold instead of inheriting an aggregate they cannot inspect.

**Fully collateralised, so the pool cannot fall short.** Coverage written never exceeds capital, and `unstake` refuses any withdrawal that would take capital below coverage written. Together these make a shortfall impossible rather than merely unlikely. A claim of `a` reduces capital by `a` and coverage written by `a`, so if `capital ≥ coverage` before the claim it still holds after, whatever order claims arrive in and however many partial claims a vault makes. This is stricter than the Ethereum system, which checks stake against the limit once at binding.

**Cover is reserved atomically with the purchase.** `open_cover` reserves the limit and charges the premium in one call and fails if capacity is short. Over-writing coverage is prevented by the contract, not by monitoring.

**The pool prices its own cover.** The premium is `limit × premium_bps / 10,000`, set at deployment, paid upfront by the vault's curator, and credited to stakers through the premium index. It is not part of `total_capital`, so it cannot be slashed.

**Many partial claims against one cover.** Each withdrawal with a shortfall is its own claim. `remaining` falls and `claimed` rises until the cover is exhausted, and `close_cover` releases whatever is left while keeping the claimed total for the record.

**Lock period.** A stake cannot be withdrawn before `locked_until`. Combined with full collateralisation, this means capital backing live cover stays until that cover is claimed or closed.

**Dust is rejected, not rounded away.** A stake into a pool that has been slashed to zero fails, and an unstake that would pay nothing fails. Rounding elsewhere is down, in the pool's favour.

**Invariants, enforced on every transition:**

```
coverage_written == Σ cover.remaining
coverage_written <= total_capital
unstake leaves total_capital >= coverage_written
claim amount <= cover.remaining
```

### 4.4 Scheduled for the milestones

The MVP covers the full happy path. Three refinements follow:

- **Loss reporting** (milestone 1). Today a loss exists as soon as a strategy fails, but stakers are slashed only when a depositor withdraws, so a staker can unstake *excess* capital in between (the capital backing cover cannot leave). The vault will report losses to the pool when they happen, so the slash lands at once and remaining cover is shared pro rata rather than first-come.
- **A notice period after the lock** (milestone 1), so a withdrawal is announced before it executes.
- **USDC pools** (milestone 2). The pool asset is already a constructor parameter.

Fractional collateralisation is not planned for this grant.

---

## 5. Contract 2 — CoveredVault

An XLM vault whose depositors are insured by a CoverPool. Source: [`contracts/covered_vault`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/covered_vault).

### 5.1 Storage and interface

```rust
#[contracttype]
pub struct VaultConfig {
    pub admin: Address,            // the curator
    pub asset: Address,            // native XLM
    pub cover_pool: Address,
}

#[contracttype]
pub struct VaultState {
    pub total_shares: i128,
    pub idle: i128,                // held by the vault itself, not allocated
}

#[contracttype]
pub struct Position {             // one persistent entry per depositor
    pub shares: i128,
    pub principal: i128,           // cost basis: deposits, reduced pro rata on withdrawal
}

// strategies: Vec<Address>, instance storage; the curator's whitelist
pub const DEDUCTIBLE_BPS: i128 = 0;   // share of a shortfall the depositor bears

// curator
fn add_strategy(env, strategy: Address);
fn buy_cover(env, limit: i128) -> i128;           // curator pays the premium
fn close_cover(env);
fn allocate(env, strategy: Address, amount: i128); // idle -> strategy

// depositors
fn deposit(env, from: Address, amount: i128) -> i128;    // -> shares
fn withdraw(env, from: Address, shares: i128) -> i128;   // -> amount paid

// views
fn nav(env) -> i128;                              // idle + Σ strategy.total_value()
fn position(env, depositor: Address) -> PositionView;   // shares, value, principal, shortfall
fn strategies(env) -> Vec<Address>;
```

Shares are internal and non-transferable, which is what makes per-address principal tracking correct. A depositor who arrives after a loss buys shares at the lower net asset value, so their principal equals their value and they carry no shortfall.

### 5.2 The payout rule, and why it runs at withdrawal

```
value     = shares × nav / total_shares                 (rounds down)
protected = principal × shares / position.shares        (rounds down)
shortfall = max(protected − value, 0)
claimable = shortfall − shortfall × DEDUCTIBLE_BPS / 10,000
paid      = value + claimable
```

`withdraw` values the shares first, then brings `value` of liquidity into the vault (idle first, then the strategies in whitelist order), then claims `claimable` from the CoverPool, then pays the depositor and updates the books. The shortfall claimed from the pool passes through the vault's balance without touching `idle`, which is accounting state rather than the live token balance.

**Why payout at withdrawal.** This is how Catalysis works on Ethereum, where the covered vault wrapper files its claim inside `_withdraw`. It needs no trigger and no oracle: a loss in a strategy lowers the vault's net asset value, and the vault's own accounting says exactly what each depositor is short. The loss event we most want to cover is the one where an oracle has already failed, so making the payout depend on a price in that moment would repeat the mistake one layer up. The payout depends on nothing but the vault's books.

**The deductible is zero, and present.** `DEDUCTIBLE_BPS` is a constant so that the withdrawal maths has the same shape as the Ethereum wrapper's `_capAndApplyDeductible`. It is not configurable in the MVP.

### 5.3 The worked example

This is the integration test and the testnet demo, with every number asserted.

| # | Step | On-chain effect |
|---|---|---|
| 0 | Alice stakes 1,000 XLM into the CoverPool | Pool capital 1,000 |
| 0 | The curator buys 50 XLM of cover, paying a 2.5% premium (1.25 XLM) | Pool reserves 50 for the vault; the premium is Alice's |
| 1 | A user deposits 100 XLM; the curator allocates 50 to Market A and 50 to Market B | Vault NAV 100 |
| 2 | Market B fails and loses 20% of its funds (10 XLM) | Vault NAV 90 |
| 3 | The user's position: value 90, protected 100, shortfall 10 | View call |
| 4 | The user withdraws. The vault pulls 90 from the strategies, claims 10 from the pool, and the pool slashes Alice by 10 | Alice's stake 1,000 → 990; remaining cover 50 → 40 |
| 5 | The user receives the full 100 XLM | Withdrawal completes |

The quiet path — no loss, the user withdraws exactly what they put in, no claim, no slashing — is the second integration test.

### 5.4 Strategies

A strategy is anything that exposes `total_value()` and `withdraw(amount)` and accepts XLM by plain transfer; the interface lives in [`contracts/common`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/common).

**MockStrategy — test and demo only.** Holds real XLM, so a simulated loss really moves tokens out and the balances on the explorer match the story. `simulate_loss(bps)` transfers that fraction of the balance to the admin. It has no yield. Never deploy it with real funds.

**Blend V2 strategy — milestone 1.** Supplies XLM to a Blend pool through `submit` and values its position as `b_tokens × b_rate`, read through Blend's public `get_reserve` and `get_positions`. Blend socializes bad debt by lowering `b_rate` (9.1), so a socialized loss shows up in `total_value()` and therefore in every depositor's shortfall, with no trigger. The `Reserve` types needed for that read are already in `contracts/common`, mirrored field for field from `blend-contracts-v2`.

### 5.5 What the vault does not do in the MVP

The Ethereum wrapper has controls the MVP deliberately omits: streaming premium charged to depositors, a per-depositor loss cap, a curator review gate between successive claims, a deposit cap, and an emergency exit that waives insurance. Section 10.5 lists these with the milestone that adds each.

---

## 6. Collateral: why the pool holds native XLM

Constraint 2.1 says an issuer's freeze and clawback rights reach balances held by our pool contract. Constraint 2.8 says a contract cannot read those rights. We checked the ledger before choosing the pool asset, and the finding decided it.

### 6.1 What we found on the ledger

We queried issuer authorization flags via Horizon for every Stellar asset we could attribute to a genuine issuer by `home_domain` and `stellar.toml`. Asset codes are permissionless — querying `BENJI` returns 23 issuers and `USDY` returns 37, nearly all impostors — so identity was established by domain, never by code. The dated snapshot, the exact queries and a script that re-runs them are in [`data/issuer-flags/`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/data/issuer-flags), so any reviewer can check the table against the live ledger in one command.

| Asset(s) | Issuer domain | revocable | clawback | immutable |
|---|---|---|---|---|
| USDC (Circle) | circle.com | **yes** | no | no |
| EURC (Circle) | circle.com | **yes** | no | no |
| BENJI (Franklin Templeton) | franklintempleton.com | yes | **yes** | no |
| USDY (Ondo) | ondo.finance | yes | **yes** | no |
| YLDS (Figure) | ylds.com | yes | **yes** | no |
| CETES, USTRY, TESOURO, GILTS, EUROB, KTB | etherfuse.com | no | no | no |
| EURC (Mykobo) | mykobo.co | no | no | no |

Every issued asset we could verify is either freezable by its issuer today, or could be made freezable with a single signature — no verified issuer has set `AUTH_IMMUTABLE`, the only flag that makes the choice permanent. USDC is freezable today. Three of the largest RWAs with attributable classic issuers — USDY ($536.4M), BENJI ($512.4M) and YLDS — can be both frozen and clawed back. One caution for anyone repeating the method: the only asset we found with `AUTH_IMMUTABLE` set was a **counterfeit** USTRY on an unrelated domain. Flags are not an identity.

### 6.2 The choice: native XLM

Native XLM has no issuer. It has no authorization flags, no freeze and no clawback, so no party can touch capital staked in the pool. It is also the deepest asset on the network, and it needs no trustline. That makes it the one asset on Stellar where the problem in 6.1 does not arise, and it is what the pool holds.

The trade-off is price correlation, and we state it plainly. XLM's price tends to fall when the whole Stellar ecosystem is under stress, so the pool's capacity measured in dollars shrinks in exactly those conditions. This is a weaker form of the wrong-way risk we criticize in Blend's backstop (1.2), and the difference is worth being precise about. BLND is correlated with the one protocol it backstops, and it sits inside an AMM pool whose bugs are the backstop's bugs. XLM is the network's base asset, has no AMM dependency, and is not the token of any covered protocol.

Cover is written in the pool's own asset. An XLM pool covers an XLM vault, and the largest single leg of the February loss was 61.25 million XLM borrowed out of the pool's XLM reserve. For cover denominated in a stablecoin, the pool asset is a constructor parameter, so a USDC pool is a redeploy rather than a code change. That pool carries Circle's freeze right, which we would disclose per vault.

### 6.3 Issued assets later

Adding issued assets needs machinery the MVP deliberately leaves out.

- **An attested flag registry.** A contract cannot read issuer flags (2.8), so a keeper writes them on-ledger with a ledger stamp. A stale or regressed attestation halts new deposits and never blocks a payout.
- **A live `authorized()` check** on the pool's own balance in the settlement path.
- **A rule that the pool's balance entry must never close.** Clawback attaches when a balance entry is created, so an entry that exists before an issuer enables clawback stays outside it.

Yield-bearing RWA capacity comes after the grant. It would need par valuation from the issuer's attestation, never a Stellar venue price: in February a treasury token with under $1 of hourly volume was marked at a manipulated price. Draw-downs would settle by redemption against the issuer, never by selling into a thin venue (2.11).

---

## 7. Loss accounting

### 7.1 Requirement

Constraint 2.3 forbids per-position writes in settlement. A claim against a pool with a thousand stakers must cost the same number of writes as a claim against a pool with ten.

### 7.2 Shares are immutable; loss moves the share value

The pool holds `total_capital` against `total_shares`, and a share is a claim on `total_capital / total_shares`. A claim reduces capital and leaves every share count untouched. The loss therefore lands on all stakers pro rata, instantly, with no per-position work and nothing to realize later. This is the slashing guarantee, and it is the implemented code:

```rust
pub fn claim(env: Env, vault: Address, amount: i128) -> i128 {
    require_vault(&env, &vault);                     // vault's own auth + whitelist
    let cfg = config(&env);
    let mut cover = cover(&env, &vault);
    if amount > cover.remaining {
        panic_with_error!(&env, Error::ClaimExceedsCover);
    }

    let mut st = state(&env);
    st.total_capital -= amount;                      // every staker slashed pro rata
    st.coverage_written -= amount;
    st.claims_paid += amount;
    cover.remaining -= amount;
    cover.claimed += amount;
    save_cover(&env, &vault, &cover);                // the vault's cover entry
    save_state(&env, &st);                           // pool state

    token::Client::new(&env, &cfg.asset)             // pool and vault balances
        .transfer(&env.current_contract_address(), &vault, &amount);
    amount
}
```

That is four state writes: the cover entry, the pool state, and the two balances the transfer touches. The count stays the same however many stakers there are. Slashing happens at claim time, and a claim happens inside a depositor's withdrawal (5.2), so stakers are slashed at the moment the vault actually pays a shortfall.

This is the mechanism Blend itself uses — `default_liabilities` socializes a loss by moving `reserve.data.b_rate`, not by touching balances — and the one ERC-4626 uses. We rejected two alternatives:

- **Epoch lists** make a dormant staker walk every loss since they joined, so the cost of an interaction grows with the pool's history.
- **A loss-per-share accumulator with share burning** has a subtler flaw. Reducing one holder's share count changes the per-share value for everyone else, so each holder's realized loss starts depending on when the others happened to interact.

Unstaking pays `shares × total_capital / total_shares`. Staking mints `amount × total_shares / total_capital`. Both round down, in the pool's favour, so rounding dust can never make the pool insolvent. Because the share price is computed from internal accounting rather than the token balance, a direct transfer into the pool cannot inflate it.

**Premium does need an accumulator,** because it is claimed separately rather than folded into share value:

- On accrual, `premium_index` advances by `amount × 10¹⁸ / total_shares`.
- A staker's earned premium is `shares × (premium_index − premium_index_entry) / 10¹⁸`. It is settled into their own entry whenever their share count changes, so claiming writes only that one entry.

The integration test replays the demo: Alice's 1,000 XLM stake is slashed exactly 10 XLM when the user withdraws, and she keeps her 1.25 XLM premium. A unit test walks three sequential partial claims against a 60/40 pair of stakers and checks the split after each one.

---

## 8. Rent, TTL and liveness

Staker positions must survive a lock period during which the staker does nothing, and depositor positions must survive for as long as the deposit sits in the vault. Both are persistent entries with independent TTLs (2.5), as is each vault's cover entry in the pool.

**Today.** Every write extends the entry it touches, so any position or cover that is interacted with stays live. `bump_position` on the pool and on the vault lets anyone extend a position entry without its owner's signature.

**Milestone 2: a funded keeper.** A standing deduction from premium funds TTL maintenance. The position exists because cover was sold, so the premium that cover generates keeps it alive. Stakers are not asked to fund rent on a position they cannot touch during the lock, and depositors in covered vaults are not charged for our storage at all.

TTL extension has no access control, so anyone may extend any entry. We will publish the keeper so third parties can run one.

**Why correctness does not depend on it.** The docs warn against relying on TTL extension for safety, and we do not. From protocol 23 an archived persistent entry included in a transaction's restore list is restored automatically when the invocation executes (2.5), so a claim against an archived position succeeds without a separate operation — it costs more in restore fees and footprint, but it does not fail. The keeper keeps claims cheap; a claim is correct whether or not it ran.

Rent pricing is itself a live network parameter rather than a constant — stellar-core's persistent rent-rate denominator changed materially at protocol 23 — so our fee model reads current values from the network rather than embedding them. Rent is an operating cost with no EVM analogue and appears as an explicit line in the delivery plan.

---

## 9. Triggers: what is decidable on-ledger

The MVP needs no trigger. The payout condition is the vault's own accounting (5.2): a depositor is paid whatever their shares are worth plus whatever that falls short of their principal. This section covers how the milestone-1 Blend strategy will value its position, which is where Blend's on-ledger state enters the design, and the oracle-derived triggers for positions that have no accounting signal, which follow mainnet (11).

### 9.1 A Blend strategy's value is accounting, not a price

*Milestone 1.* The loss event we most want to cover is the one where the oracle has already failed, so a strategy's value must come from the covered protocol's own books.

**On Blend, the observable has to be chosen carefully, because the obvious one disappears at the moment of the event.** Bad debt is not a readable field: `bad_debt(user)` is a state-changing transition whose documented behaviour is that the backstop "will take over the debt," and if the backstop is itself unhealthy it "will default it's remaining debt." Pending bad debt therefore shows up as d-token liabilities sitting on the backstop address — but socialization, in `default_liabilities`, *removes* those liabilities and reduces `reserve.data.b_rate`. A strategy that watched backstop liabilities would see its signal go to zero exactly when the covered event occurred.

So the two states play different roles:

- **`b_rate`** is what the strategy values itself by. A supplier's claim on a Blend reserve is their bTokens × `b_rate`, so `total_value = b_tokens × b_rate / 10¹²`, read through Blend's public `get_reserve` and `get_positions`. When Blend socializes a loss it lowers `b_rate`, the strategy is worth less, and every depositor's shortfall appears at withdrawal with no further mechanism. The backstop acts as the deductible without any parameter: Blend lowers `b_rate` only when it defaults debt that the backstop could not absorb, so Catalysis sits behind the backstop, never in place of it.
- **Backstop liabilities** are an early warning: pending bad debt that has not yet been socialized. Milestone 1 uses them to drive loss reporting (4.4) so stakers cannot exit ahead of a loss that is visible but not yet booked. They never pay out on their own, and they are poke-dependent — debt reaches the backstop only when someone calls `bad_debt(user)`, and that call panics while a liquidation or bad-debt auction is open — so their absence proves nothing.

```rust
// milestone 1 — the Blend strategy's total_value, through Blend's public interface (2.8)
let reserve   = BlendPoolClient::new(&env, &pool).get_reserve(&asset);
let positions = BlendPoolClient::new(&env, &pool).get_positions(&env.current_contract_address());
let b_tokens  = positions.supply.get(reserve.config.index).unwrap_or(0);
b_tokens * reserve.data.b_rate / 1_000_000_000_000
```

The `Reserve`, `ReserveConfig` and `ReserveData` types in `contracts/common` mirror `blend-contracts-v2` field for field, so the same read works against a real Blend pool on testnet.

Four properties of Blend's read surface shape the strategy:

- **Blend does not expose `get_backstop()`.** The pool's public interface is `get_config`, `get_admin`, `get_reserve_list`, `get_reserve`, `get_positions`, `get_reserve_emissions`, `get_user_emissions`, `get_auction`, and a contract cannot read another contract's storage (2.8). The backstop address is therefore a strategy parameter verified off-chain against the pool's deployment.
- **`Positions` is keyed by reserve index, not asset.** `supply: Map<u32, i128>` and `liabilities: Map<u32, i128>`, so the index is resolved first via `get_reserve(asset).config.index` or `get_reserve_list()`.
- **These reads are point-in-time.** `Reserve::load()` updates to the current timestamp and auction data scales with elapsed ledgers, so nothing is cached across ledgers.
- **`pool_data` on the backstop calls into the Comet AMM** for total supply and balances, so the Comet contract lands in the footprint of any read that uses it.

The strategy calls Blend's `submit` to supply and withdraw and nothing else. For accuracy on a related point: Blend V2 bounds borrowing through `max_util` and a per-asset `supply_cap` rather than a per-asset borrow cap, so any risk-parameter scoring we publish names those fields.

**On DeFindex the shape differs and the read path is not free of side effects.** `fetch_total_managed_funds()` returns a per-asset idle-versus-invested breakdown; there is no `total_assets()` and no scalar share price, so a DeFindex strategy computes its value from that basket and the vault's `total_supply`. Critically, `get_asset_amounts_per_shares()` internally calls `fetch_total_managed_funds(e, true)`, and that flag — per DeFindex's own comment — "ensures that strategy reports are updated and new fees are locked." Calling it locks fees inside someone else's vault. **We therefore never call it from a permissionless entrypoint.** Even `fetch_total_managed_funds()` bumps the vault's instance TTL, so a DeFindex valuation is a read-write transaction whose footprint includes the vault and its strategy entries and scales with assets × strategies rather than being constant. That cost is disclosed here and priced into those strategies rather than discovered by PaltaLabs later.

**DeFindex does not implement SEP-56.** SEP-0056 is a Draft (v0.1.2, November 2025) authored by OpenZeppelin and Sentinel specifying a 17-function single-asset `TokenizedVault` trait; DeFindex's `VaultTrait` is multi-asset by construction and implements none of `total_assets`, `convert_to_shares`, `convert_to_assets` or the `preview_*`/`max_*` family. An integration targets DeFindex's own interface. We track SEP-56 for the generic strategy and treat its Draft status as an integration risk rather than a foundation — OpenZeppelin's shipped `FungibleVault` already diverges from the draft's trait naming. SEP-40 is likewise a Draft (v0.1.0), and we treat it the same way.

### 9.2 Oracle-derived triggers, and where they are not available

*After the grant.* Price- and NAV-type triggers exist for cover on positions with no accounting signal, where the shortfall cannot be read from a vault's books. Four conditions must all pass.

**Two independent feeds must agree.** We read Reflector's Stellar DEX feed and its external CEX/DEX feed and require divergence within `threshold_bps`. Both feeds' entries fit comfortably in one footprint — roughly 48 price records for two 2-hour windows, plus instance and code entries, against `txMaxFootprintEntries` of 400 — and an attacker cannot skip the check by omitting a feed, because touching an entry outside the declared footprint traps. Each feed quotes in its own `base()` with its own `decimals()`, so the adapter normalizes both before comparing, and each asset entry carries its own TTL via `expires(asset)`, which is checked before use.

**The window is built consumer-side and is visibly longer than the one that failed.** Reflector exposes no TWAP (2.6), so the adapter pulls `prices(asset, records)` and time-weights them itself. At the default 5-minute resolution a 2-hour window is twenty-four records, against a 24-hour retention ceiling. We default to 2 hours for deviation triggers: forensics of the February incident show oracle price entries five minutes apart, so a window measured in hours raises the cost of sustaining a manipulated price by orders of magnitude.

*On the window that failed:* Halborn describes the feed as a 15-minute VWAP, while Reflector's public Pulse feeds default to 5-minute resolution — which suggests the failing oracle may not have been the canonical Pulse DEX feed but a differently provisioned deployment. We do not resolve this from secondary sources; the adapter reads `resolution()` from the specific contract it is wired to rather than assuming either value.

**Depth, where it is readable, must clear a floor.** AMM reserves are contract state and can be read; the classic orderbook cannot be (2.7). Aquarius is effectively the only venue with volume — Soroswap shows $0 over both 24 hours and 30 days on DefiLlama — so in practice the depth floor applies where an Aquarius pool exists and nowhere else.

**Staleness is checked.** SEP-40's `PriceData` carries only `price` and `timestamp` — no confidence interval, no round id — so timestamp age against the feed's `resolution()` is the only staleness signal available, and an observation older than a configured bound is not actionable.

**Where the gates are unavailable, no price trigger is offered.** This is the honest and load-bearing case, not a corner case. A tokenized treasury with sub-$1 hourly volume on its only venue will have no Aquarius pool, and Reflector feeds carry an explicit per-asset list, so an asset the external feed does not quote returns `None` rather than a comparison price. Both oracle-side gates then have nothing to work with. The rule is **fail-closed**: where the external feed does not quote the asset, no price- or NAV-type trigger is available and cover is written on accounting only. Confirming per-asset coverage against `assets()` on the external feed is the first task of that work, and it is why accounting (5.2, 9.1) is the primary path rather than a fallback.

```rust
#[contracttype]
pub struct Observation {
    pub value: i128,              // time-weighted, feed_a, normalized
    pub cross_value: Option<i128>,// same window, feed_b; None if unquoted
    pub divergence_bps: Option<u32>,
    pub depth: Option<i128>,      // AMM reserves where a venue exists
    pub records: u32,
    pub oldest_ledger: u32,
    pub newest_ledger: u32,
}

pub trait TriggerModule {
    fn observe(e: Env, spec: TriggerSpec) -> Observation;
    fn is_actionable(e: Env, obs: Observation, spec: TriggerSpec) -> bool;
    fn evaluate(e: Env, spec: TriggerSpec, obs: Observation) -> bool;
}
```

`is_actionable` and `evaluate` are deliberately separate. The first asks whether the observation may be trusted; the second whether it meets the payout condition. A manipulated observation fails the first and never reaches the second, and an unquoted one fails it too.

---

## 10. Security

### 10.1 Threat model

| Threat | Mitigation | Status |
|---|---|---|
| Payout redirected, by us or anyone | `claim` pays the calling vault against its own cover; no recipient parameter; no claims role exists (4.2) | Implemented |
| Payout larger than the cover bought | Pool caps every claim at the vault's remaining cover (4.3) | Implemented |
| Unauthorized caller opens, claims or closes cover | Vault-facing pool functions require the vault's own auth and the admin's whitelist (4.2) | Implemented |
| A depositor paid more than they are owed | Payout is `max(value, protected)`; principal is reduced pro rata on every withdrawal (5.2) | Implemented |
| Pool unable to pay a valid claim | Full collateralisation; capital backing cover cannot be unstaked (4.3) | Implemented |
| Capacity over-writing | Reservation atomic with purchase; `coverage_written <= total_capital` (4.3) | Implemented |
| Issuer freezes or claws back pool capital | Pool holds native XLM, which has no issuer (6.2) | Implemented |
| Allowance revoked when a loss looks likely | Custody-lock staking; no `transfer_from` path (4.3) | Implemented |
| Settlement cost grows with stakers | Loss absorbed by share value; four writes, constant in staker count (7.2) | Implemented |
| Pool share price inflated by a direct transfer | Share price computed from internal accounting, not token balance (7.2) | Implemented |
| Vault share price inflated by donating to a strategy | Zero-share mints rejected; virtual share offset or minimum first deposit to follow (10.5) | Partial |
| Rounding drift | Mint, redeem and shortfall maths round down, in the contract's favour; dust operations rejected (4.3, 5.2) | Implemented |
| Oracle manipulation to suppress or force a payout | The payout reads no price; it is the vault's accounting (5.2) | Implemented |
| Cross-contract reentrancy | Blocked by the host (2.9); the vault calls out, nothing calls back in (4.2) | Implemented |
| Staker flight ahead of a known loss | Lock and full collateralisation today; loss reporting and a notice period (4.4) | Milestone 1 |
| Cover exhausted, later withdrawers unpaid | Cover sized above maximum loss today; pro-rata shortfall accounting (10.5) | Milestone 1 |
| Curator allocates to a bad strategy | Curator whitelists strategies; depositors trust the curator, as on Morpho; allocation limits (11) | Milestone 2 |
| Position archived when a withdrawal or claim lands | Protocol-23 automatic restore; interactions extend TTL today, keeper in milestone 2 (8) | Partial |
| Admin key compromise | 10.3 | Before mainnet |

### 10.2 Testing

83 tests, run by `scripts/check.sh` together with `cargo fmt`, `cargo clippy -D warnings` and the `stellar contract build` of every contract; the same steps are in the GitHub Actions workflow.

- **34 CoverPool unit tests.** Share minting, the lock boundary, capital backing cover being unable to leave, exact capacity and one over, premium split by shares and not retroactive to late stakers, premium kept by stakers who leave, premium rounding and dust conservation, full and partial claims, three sequential partial claims against a 60/40 pair, two vaults sharing one pool's capacity, a staker leaving between claims, share price after an odd slash, one-unit stakes, the drained pool, and a scripted sequence that checks every invariant in 4.3 after each step.
- **39 CoveredVault unit tests.** Share minting at par and at a lower NAV, NAV across idle and two strategies, the demo scenario step by step, withdrawals with no loss, a partial loss, a total loss and a profit, partial withdrawals of different sizes sharing a loss pro rata, five depositors through a loss, depositors at different entry prices, a late depositor through two losses in either order, the same depositor depositing twice, interleaved partial and full exits, cover exhaustion leaving state unchanged, stroop-level rounding, zero-share and zero-payout rejection, and a scripted life of the vault that checks the books after every step.
- **8 MockStrategy unit tests.** Value, withdrawal, the demo's 20% loss, total loss, bounds and authorization.
- **2 end-to-end tests** across all three contracts: the worked example in 5.3 with every number asserted and every balance reconciled, and the quiet path.

The scenario tests were derived from the Ethereum Catalysis suites (the covered vault wrapper, claim manager, cover pool and premium manager), recomputed for the MVP's simpler rules. The testnet script has run the worked example end to end against the deployed contracts.

Planned for milestone 1:

- property tests on the pool and vault invariants across randomized sequences
- the same scenario against a real Blend V2 pool on testnet through the Blend strategy

### 10.3 Admin authority and its limits

The pool admin can whitelist vaults and upgrade the pool. It cannot alter a vault's cover, redirect or block a claim, withdraw staked capital, or change share value other than through a claim.

The vault admin is the curator. The curator can whitelist strategies, allocate idle deposits to them, buy and close cover, and upgrade the vault. The curator cannot touch a depositor's principal or shares, block a withdrawal, or take deposits out of the vault other than into a whitelisted strategy. Allocation is the curation role, and depositors trust the curator's judgement about strategies, exactly as depositors in a Morpho vault do; milestone 2 adds per-strategy allocation limits.

The mock strategy's admin can simulate a loss. That is a demo harness and not part of the product.

Underwriting terms — the premium rate and the lock — are fixed when the pool is deployed. The upgrade path is each admin's one broad power, so before mainnet it moves behind a multisig with a timelock, both published on deployment.

### 10.4 Audit and licensing

Sigma Prime led our Ethereum audits (Core, March 2026; Coverage, April 2026; combined, May 2026), with Certora on the covered vault (March 2026). All four reports are public and linked in section 14.

For the Soroban contracts we engage a Soroban-experienced auditor, with scope agreed before implementation completes and remediation closed before mainnet. SCF audit credits cover the cost, and no audit expense appears in the budget. The Soroban contracts are published under Apache-2.0.

### 10.5 Known MVP limitations

Each of these is a deliberate simplification of the Ethereum design, documented rather than fixed in the MVP, with the milestone that addresses it.

- **First-come compensation if cover is too small.** Payout at withdrawal means that if total shortfall exceeded the vault's remaining cover, early withdrawers would be made whole and later ones not; a withdrawal whose shortfall exceeds the remaining cover fails outright. The MVP avoids this by sizing cover above the maximum loss. This matches Ethereum, where remaining cover is also first-come at withdrawal. Milestone 1: record the shortfall when a loss is reported and share remaining cover pro rata.
- **Stakers can exit ahead of an unclaimed loss.** A loss exists as soon as a strategy fails, but stakers are slashed only when a depositor withdraws, so a staker can unstake *excess* capital in between. Capital backing cover cannot leave, so payouts are always funded; the remaining stakers carry more of the loss. Milestone 1: loss reporting (4.4).
- **No cover expiry and no streaming premium.** The curator pays upfront and cover lasts until `close_cover`. On Ethereum premium streams from depositor principal and is split between a platform fee, a pool fee and the restakers; the MVP has no fee split. Milestone 1.
- **No curator review gate between claims.** On Ethereum every approved claim requires the curator to approve the next before another loss-making withdrawal succeeds. Milestone 1.
- **Deductible fixed at zero; no per-depositor loss cap.** Milestone 1 makes both configurable.
- **Cover can be topped up.** Ethereum policies have a fixed limit; here `open_cover` adds to remaining cover, which is simpler for the demo.
- **Share inflation by donation is only partly mitigated.** Because principal is protected in full, an attacker who donates XLM to a strategy to inflate the share price could make the pool pay a later depositor's rounding loss. The MVP rejects zero-share mints, which is the classic first step. Milestone 1: virtual share offset or a minimum first deposit, as in the Ethereum wrapper's decimals offset.
- **A drained vault or pool rejects new money** rather than recovering. Redeploy.
- **Withdrawals drain strategies in whitelist order**, not pro rata. Milestone 1.
- **Shares are non-transferable.** By design; a transferable share would need principal to travel with it.
- **The admin can upgrade the contracts.** Multisig and timelock before mainnet (10.3).
- **The mock strategy's admin triggers losses.** Demo only.

## 11. Delivery

Three milestones. The final one is mainnet launch. Every deliverable has a binary completion test, stated in the third column.

| Milestone | Scope | Done when | Target |
|---|---|---|---|
| 1 — Real yield, real losses | **Blend V2 strategy** on testnet: supplies and withdraws through Blend's `submit`, values its position through `b_rate` (5.4, 9.1). **Loss reporting** at strategy level: the vault reports a loss to the pool when it happens, the pool records the shortfall, and remaining cover is shared pro rata rather than first-come (4.4, 10.5). **Staker notice period** after the lock. **Cover expiry and streaming premium** charged from vault yield instead of upfront by the curator. **Property tests** on the pool and vault invariants across randomized sequences. | The demo scenario runs on testnet against a live Blend V2 pool, with the loss coming from a real `b_rate` decline rather than a mock. A reported loss slashes stakers before any depositor withdraws. Two depositors with a combined shortfall above the remaining cover are each paid the same fraction. Property tests run in `scripts/check.sh` and CI. | 31 Jan 2027 |
| 2 — Curation and operations | **Multiple Blend markets** per vault with curator **allocation limits** enforced on-chain (10.3). **Keeper-based TTL maintenance** funded from premium, published so third parties can run it (8). **USDC vault and pool** alongside XLM, with the issuer freeze right disclosed (6.2). **STRIDE threat model and on-chain monitoring plan**, published. **Audit engaged**, contract signed and scope frozen. | A vault on testnet holds positions in two live Blend markets under limits the contract enforces. A keeper has kept every position alive through a full TTL cycle on testnet. The demo runs end to end on a USDC vault and pool. The threat model and monitoring plan are in this repository. The audit firm and start date are named in the tranche report. | 28 Feb 2027 |
| 3 — Mainnet | **Audit remediation** with the report published. **Multisig and timelock** on every admin function (10.3), with `migrate()` for future upgrades (2.10). **Mainnet deployment.** **First live covered vault** with third-party stake in the pool and cover bought. **SDK and integration docs** for curators and stakers. | All audit findings closed and the report linked from this document. Mainnet contract IDs published, every admin held by the multisig behind the timelock. One covered vault live on mainnet, allocated to a Blend market, with stake from at least one address outside the team and cover open on it. The SDK is published with docs and used by the demo. | 31 Mar 2027 |

Oracle-derived triggers (9.2) and a DeFindex strategy (9.1) follow mainnet and are not in this grant's scope. They are specified here because they shape the strategy interface, and because a reviewer should see what the accounting-only design deliberately excludes.

Build starts in the first week of January 2027 and completes within the first quarter.

The published #46 calendar runs:

- submission closes 8 November 2026
- panel review 11 to 18 November
- community vote 23 November to 3 December

Notification, information collection and KYB follow the vote, so tranche #0 may arrive while milestone 1 is under way; we start on our own balance sheet rather than wait for it. Each tranche completion form falls well inside the 90-day window that runs from receipt of the preceding payment.

**The critical path is the audit, not the engineering.** Mainnet by 31 March needs an audit slot, the audit itself and remediation to fit inside March. The audit is therefore engaged during milestone 2 with scope frozen at the end of February. Milestone 3 adds no new contract functionality — only remediation, deployment and the first live vault.

**Milestone 1 starts partly delivered.** The Ethereum implementation of this system is shipped and audited, so the Soroban work is a reimplementation against a known specification rather than a design exercise. And the full happy path is built before this submission: staking, cover purchase, deposits and curator allocation, a strategy loss, payout at withdrawal with pro-rata slashing, premium distribution and unstaking, with 83 tests, a local and CI check, and a testnet demo whose contract IDs are in the README.

---

## 12. Open questions

1. **Issued assets as capacity.** Every verified issued asset on Stellar is freezable by its issuer today, or could be made so with one signature (6.1). Regulated issuers keep those rights for compliance, so that is unlikely to change. Adding USDC or yield-bearing RWA capacity therefore means pricing and disclosing issuer risk, not designing it away. Tokens issued as Soroban contracts carry no issuer flags at all; their freeze behaviour lives in each contract's own admin logic, and needs assessing case by case.
2. **Which oracle the YieldBlox pool actually read, and at what resolution** (9.2). Halborn's 15-minute characterisation and Reflector's 5-minute default cannot both describe the same feed. We read `resolution()` from the wired contract rather than assuming either.
3. **Whether the YieldBlox backstop was fully drawn or merely insufficient.** No public source we found states it. It bears on how often `b_rate` actually falls in practice, and it is a question for Script3 rather than an inference.
4. **Premium calibration for Stellar.** Our Ethereum pricing is calibrated on crypto-native collateral. Thin venue liquidity (2.11) and XLM-denominated capacity (6.2) call for different parameters. The MVP charges a flat rate set at deployment.
5. **Live network settings.** Every resource limit and rent parameter here is a network setting rather than a constant (2.3, 8). SLP-0004 and SLP-0005 both read Final in the protocol repository; we read live values from the network at build time.

---

## 13. Team

Two co-founders, both protocol engineers.

**Abhishek Kumar — CEO.** Ex-Product at Obol (DV Labs), protocol engineering at Nethermind, previously Amazon and Atlassian. Ethereum Proof-of-Stake, staking infrastructure, DeFi.

**Dhruv Bodani — CTO.** Ex-Tech Lead for L2s at Obol (DV Labs), Senior Engineer at Offchain Labs (Arbitrum), protocol engineering at Nethermind, previously Visa and Atlassian. Distributed and P2P systems, Ethereum PoS, DeFi.

We designed, audited and shipped Catalysis to Ethereum mainnet — vault-native risk coverage, with coverage operating over a Tier-1 curator's vault on Morpho and underwriting capacity sourced from EigenLayer and Symbiotic. Audited by Sigma Prime and Certora. Both founders co-authored *Economic Security of Multiple Shared Security Protocols* (Marble 2025, Imperial College Brevan Howard Centre for Financial Analysis), on the economic security of restaking across shared security providers.

---

## 14. Sources

Claims were verified against primary sources on 25 September 2026. Three claims in circulation are **not** used here because they did not survive verification: that Stellar TVL fell approximately 60% to $98M in late August 2026; that the manipulated feed is confirmed to have used a 15-minute VWAP window; and that DeFindex conforms to SEP-56.

**Stellar protocol and platform**
Stellar Asset Contract — https://developers.stellar.org/docs/tokens/stellar-asset-contract · Controlling asset access — https://developers.stellar.org/docs/tokens/control-asset-access · Persisting data (TTL extension, reliance warning) — https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/persisting-data · State archival — https://developers.stellar.org/docs/learn/fundamentals/contract-development/storage/state-archival · Fees, resource limits, metering — https://developers.stellar.org/docs/learn/fundamentals/fees-resource-limits-metering · Upgrading contracts — https://developers.stellar.org/docs/build/guides/conventions/upgrading-contracts · CAP-35 — https://github.com/stellar/stellar-protocol/blob/master/core/cap-0035.md · CAP-58 — https://github.com/stellar/stellar-protocol/blob/master/core/cap-0058.md · CAP-66 — https://github.com/stellar/stellar-protocol/blob/master/core/cap-0066.md · SLP-0004 — https://github.com/stellar/stellar-protocol/blob/master/limits/slp-0004.md · SLP-0005 — https://github.com/stellar/stellar-protocol/blob/master/limits/slp-0005.md

**Standards and libraries**
SEP-40 (Draft) — https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0040.md · SEP-41 (Draft) — https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md · SEP-56 (Draft v0.1.2) — https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0056.md · OpenZeppelin Stellar Contracts — https://github.com/OpenZeppelin/stellar-contracts

**Oracles**
Oracle providers and feed IDs — https://developers.stellar.org/docs/data/oracles/oracle-providers · Reflector contract — https://github.com/reflector-network/reflector-contract

**Blend, DeFindex, Comet**
Blend contracts V2 — https://github.com/blend-capital/blend-contracts-v2 · Blend whitepaper — https://docs.blend.capital/blend-whitepaper · Backstopping — https://docs.blend.capital/users/backstopping · Blend FAQ — https://docs.blend.capital/users/general-faq · Blend V2 Certora formal verification — https://code4rena.com/audits/2025-02-blend-v2-audit-certora-formal-verification · DeFindex — https://github.com/paltalabs/defindex · DeFindex docs — https://docs.defindex.io · Comet contracts v1 — https://github.com/CometDEX/comet-contracts-v1

**Incident reporting**
rekt.news — https://rekt.news/yieldblox-rekt · Halborn — https://www.halborn.com/blog/post/explained-the-yieldblox-hack-february-2026 · BlockSec — https://blocksec.com/blog/yieldblox-dao-incident-on-stellar-oracle-misconfiguration-enabled-a-10m-drain · Bankless — https://www.bankless.com/read/news/lending-market-blend-suffers-10m-exploit · DailyCoin (August Comet report, single-source) — https://dailycoin.com/stellars-tvl-peak-unloads-60-can-xlm-hold-its-ground

**Market data**
DefiLlama Stellar — https://defillama.com/chain/Stellar · DefiLlama Stellar DEXs — https://defillama.com/dexs/chain/stellar · DefiLlama Blend — https://defillama.com/protocol/blend · rwa.xyz Stellar — https://app.rwa.xyz/networks/stellar · Stellar RWA dashboard (Dune, maintained by Stellar) — https://dune.com/stellar/rwas

**Catalysis**
Issuer-flag snapshot, queries and re-run script — `data/issuer-flags/` in https://github.com/0xcatalysis/catalysis-soroban · Mainnet contracts — https://docs.catalysis.network/docs/mainnet/ · Audit reports — https://docs.catalysis.network/docs/audits/ · Research — https://arxiv.org/abs/2505.03843

**SCF**
SCF #46 round page — https://communityfund.stellar.org/awards/recxrSMYwAl8vcglg

*Research synthesis and drafting for this document were AI-assisted. The protocol design, the architectural decisions and all technical claims are the team's own, and each is individually sourced above.*
