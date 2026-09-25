# Catalysis on Soroban — Technical Architecture

**Underwritten bad-debt coverage for Stellar credit markets**

Version 1.2 · 25 September 2026 · Prepared for Stellar Community Fund #46, Build Award (Open Track)
Authors: Abhishek Kumar, Dhruv Bodani · Catalyst Labs LLC FZ

---

## 0. Scope

This specifies the on-ledger design of a coverage protocol for Stellar: the contracts, their storage, their interfaces, the accounting method, the trigger conditions, and the integration surfaces against Blend V2 and DeFindex. It is the design the SCF #46 build executes against.

It does not restate our Ethereum implementation. The Soroban system differs from it in its capital model, its loss accounting and its trigger construction, and section 2 gives the specific properties of Stellar that force each difference.

Every figure carries a source. Where sources disagree the range is given rather than a point estimate, and where a claim could not be verified from a primary source it is marked and the design does not rest on it.

**Implementation status.** The happy path described in sections 3 to 7 is implemented, tested and deployable today at https://github.com/0xcatalysis/catalysis-soroban: CoverPool, Policy, the Blend adapter, 14 passing tests including an end-to-end bad-debt scenario, and a script that runs the same scenario on testnet. Anything scheduled for a milestone is marked as such where it appears.

**Reading order for a reviewer with limited time:** section 2 (the constraints), section 4 (the staked pool and its slashing guarantee), section 6 (why the pool holds native XLM, which has no Ethereum analogue), section 9.1 (what is actually readable from Blend).

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

A **second-loss layer**: exogenous underwriting capital committed to named pools and vaults under contract, with payout conditions encoded on-ledger and settlement executable by anyone who can observe that the conditions are met. Capital that is not the covered protocol's own token. No claims committee, no governance vote, no discretionary compensation, no validator coordination.

We have built this once, on Ethereum, audited and live. That is the reason to fund the Stellar version and not the design for it — what follows is a different system in the ways Stellar requires.

---

## 2. The constraints that shape this design

Each item below is a property of Stellar or Soroban that changed a decision. Removing any one of them breaks the design.

**2.1 Issuer authorization flags reach inside contract balances.** An asset issuer's authorization state applies to a balance held by a Soroban contract, not only to classic trustlines. The Stellar Asset Contract documentation is explicit: deauthorization "is true for both the trustline balances of `Address::Account` and contract balances of `Address:Contract`," and "a balance can only be clawed back if the issuer account had the `AUTH_CLAWBACK_ENABLED_FLAG` set when the balance was created." A contract address is not a shelter. For a coverage protocol this is existential — capital an issuer can freeze is not capacity — and it has no Ethereum analogue. Section 6 is the response.

**2.2 There is no restaking, no shared security, and no slashable external capital.** On Ethereum our capacity comes from an existing market: restakers hold slashable EigenLayer and Symbiotic delegations, and someone else built the slashing rail. Stellar has no equivalent. Capacity must be raised, committed, priced and drawn natively. This is the largest piece of new engineering in this build.

**2.3 Footprint limits bound the shape of loss accounting, not just its cost.** Soroban resource limits are on-ledger network settings set by validator vote, not constants. SLP-0004 (Final, 12 January 2026) raised per-transaction write entries from 50 to 200, per-ledger writes from 500 to 1000, and footprint entries from 100 to 200; SLP-0005 (Final, 18 February 2026) raised footprint entries to 400. Protocol 23 (CAP-0066) also changed *what* is metered: `txMaxDiskReadEntries` counts disk reads only, in-memory reads carry no entry- or byte-based fee and no byte limit but remain bounded by an entry-count limit, and `txMaxFootprintEntries` caps read-only and read-write entries combined. The consequence is unchanged by the increase: **settlement must never iterate over staker positions.** A pool with a thousand stakers cannot be settled by touching a thousand entries, at any limit. Section 7 makes settlement constant in staker count.

**2.4 There is no multicall.** stellar-core enforces one operation per Soroban transaction and rejects mixing Soroban with non-Soroban operations. Batching happens *inside* a single invocation, in a loop bounded by the live footprint limit — so batch sizes are contract parameters read from storage, never compile-time constants.

**2.5 Persistent state archives, and the fix is bundled, not separate.** Every capacity position and every policy is a persistent ledger entry with its own TTL; minimum and maximum TTLs are network parameters. Two properties we rely on: TTL extension has no access control — "any user may invoke `ExtendFootprintTTLOp` on any LedgerEntry" — so a keeper can maintain positions without a signature from their owner; and from protocol 23 archived persistent entries in a transaction's restore list are restored automatically when the invocation executes, so a claim against an archived entry needs no separate `RestoreFootprintOp` (which must be the only operation in its transaction and would therefore be unusable mid-settlement). The docs also warn that "TTL extensions should never be relied on for functionality or safety." Section 8 takes that seriously.

**2.6 Reflector gives prices, not price analytics, and its resolution is coarse.** Reflector's public Pulse feeds default to a 5-minute resolution with a 24-hour default history retention period, and expose SEP-40's `price`, `prices` and `lastprice` — **no TWAP function**. Time weighting is computed consumer-side from `prices(asset, records)`, and retention bounds any window at 24 hours. Reflector's Beam product updates faster but charges an XRF fee per read and requires the caller to authorize a burn, putting a token dependency and a sub-invocation in the read path; we design against Pulse. Both `resolution()` and `history_retention_period()` are contract-readable and we read them rather than assuming the defaults.

**2.7 A Soroban contract cannot read the classic DEX orderbook.** This is the constraint an Ethereum-native design would get wrong. The venue whose thinness enabled the February manipulation is not introspectable from inside a contract: there is no host function returning SDEX order book depth. What *is* readable on-ledger is AMM pool reserves and a second, independent Reflector feed — the external CEX/DEX feed (`CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LZLFTK6JLN34DLN`) alongside the Stellar DEX feed. Cross-feed divergence is the only depth proxy Stellar makes available to a contract. Section 9.2 specifies it, including the case — common for RWAs — where the second feed does not quote the asset at all.

**2.8 A Soroban contract cannot read another contract's storage, or a classic account's flags.** `e.storage()` is namespaced to the invoking contract; there is no arbitrary-ledger-entry host function, and the host exposes no account-flag accessor. Everything we learn about Blend, DeFindex or an issuer comes through a public function that contract chooses to expose, or through an attestation written on-ledger by a keeper. Sections 6.3 and 9.1 are both shaped by this, and both say so where a naive design would assume otherwise.

**2.9 Cross-contract reentrancy is blocked by the host.** `rs-soroban-env` defaults every external contract call to `ContractReentryMode::Prohibited`, and detection scans the whole context stack, so indirect reentry (A→B→A) is caught. This removes a class of attack and also a pattern: no callback-driven settlement, no reentrant vault hooks. Settlement resolves state fully before any external transfer.

**2.10 Upgrades replace Wasm and do not re-run constructors.** `update_current_contract_wasm` preserves address and storage, and CAP-0058 states that on code update a contract "is not considered created and thus constructor won't be called," so a constructor in the current Wasm carries no guarantee of having run. Contracts therefore carry an explicit, idempotent, version-gated `migrate()`. There is no delegatecall and contract size is capped, so the system is several contracts rather than one.

**2.11 Network liquidity is thin and concentrated in a single venue.** Aquarius carries about 98.7% of DefiLlama-tracked Stellar DEX volume — $5.28M of $5.35M over 24 hours, $69.24M of $70.87M over 30 days, as of 25 September 2026. **No part of this system may assume it can liquidate collateral on-ledger at size.** Claims settle in the asset the pool already holds. We never sell collateral into a Stellar venue to fund a payout.

---

## 3. System overview

Four contract roles. Three are implemented in the repository today — CoverPool, Policy and the Blend observation adapter — together with a mock Blend pool that exists only for tests and the testnet demo. The fourth, an oracle trigger module for positions with no accounting signal, is milestone 2.

```
┌───────────────────────────────────────────────────────────────────────┐
│ STELLAR / SOROBAN                                                     │
│                                                                       │
│  COVERED PROTOCOL                    CATALYSIS                        │
│  ┌──────────────────┐               ┌──────────────────────────────┐  │
│  │ Blend V2 pool    │◄─get_reserve──│ (3) Blend adapter            │  │
│  │  reserve b_rate  │               │     read-only                │  │
│  └──────────────────┘               └──────────────┬───────────────┘  │
│                                                    │ b_rate           │
│  ┌──────────────────┐   buy cover   ┌──────────────▼───────────────┐  │
│  │ Vault / lender   │──────────────►│ (2) Policy                   │  │
│  │ (beneficiary)    │               │  baseline · trigger · payout │  │
│  └────────▲─────────┘               └──────────────┬───────────────┘  │
│           │                        reserve ·       │ settle           │
│           │ payout                 premium         ▼                  │
│           │                         ┌──────────────────────────────┐  │
│           └─────────────────────────│ (1) CoverPool                │  │
│                                     │  staked XLM · shares ·       │  │
│                                     │  premium index               │  │
│                                     └──────────────▲───────────────┘  │
│                                                    │ stake / unstake  │
│                                     ┌──────────────┴───────────────┐  │
│                                     │ Stakers (native XLM)         │  │
│                                     └──────────────────────────────┘  │
│                                                                       │
│  milestone 2:  (4) oracle trigger module ◄── Reflector SEP-40 feeds   │
└───────────────────────────────────────────────────────────────────────┘
```

Capital custody, policy state, trigger evaluation and settlement all execute on Stellar; nothing in the settlement path leaves the ledger. The adapter calls only Blend's public read function and holds no authority over the pool it observes.

**The flow.** Stakers stake XLM into a CoverPool and receive shares under a lock period. A vault or lender buys cover on its supply position in a Blend reserve: the Policy contract records the reserve's `b_rate` as a baseline, reserves the policy's limit in the CoverPool together with the beneficiary, and charges premium, which goes to stakers. If Blend later socializes bad debt across that reserve's suppliers, it lowers `b_rate`. Once the decline passes the trigger threshold, anyone may call `settle`: the covered loss — `covered_amount × decline`, capped at the limit — is paid to the beneficiary recorded at reservation, and every staker is slashed pro rata through the pool's share value. If nothing happens before the policy ends, anyone may call `expire` and the reserved capacity returns to the pool.

**Policies are entries in one Policy contract, not one contract instance each.** A factory-per-policy design would put a contract instance and its own rent under every policy, and Soroban's instance storage shares the contract's TTL, so a thousand policies would mean a thousand instances to keep alive. A single contract holding policies as persistent entries keyed by `u64` costs one entry per policy.

---

## 4. Contract 1 — CoverPool

A staked capital pool that underwrites coverage. This is the Stellar-native replacement for the EigenLayer and Symbiotic capacity our Ethereum system draws on (2.2): not restaking, but a plain capital pool with slashing on payout, which is what the coverage layer actually needs. Source: [`contracts/cover_pool`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/cover_pool).

### 4.1 Storage

```rust
#[contracttype]
pub struct PoolConfig {
    pub admin: Address,
    pub asset: Address,        // Stellar Asset Contract of the staked asset: native XLM
    pub lock_ledgers: u32,     // minimum stake duration
}

#[contracttype]
pub struct PoolState {
    pub total_capital: i128,   // asset units backing all shares
    pub total_shares: i128,
    pub coverage_written: i128,// sum of reserved policy limits
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
pub struct Reservation {       // one persistent entry per live policy
    pub limit: i128,
    pub beneficiary: Address,
}
```

### 4.2 Interface

```rust
// stakers — each requires the staker's own auth
fn stake(env, from: Address, amount: i128) -> i128;          // -> shares
fn unstake(env, from: Address, shares: i128) -> i128;        // -> amount
fn claim_premium(env, from: Address) -> i128;

// policy-facing — each requires auth from the registered Policy contract
fn reserve(env, policy_id: u64, limit: i128, beneficiary: Address);
fn release(env, policy_id: u64);
fn accrue_premium(env, policy_id: u64, from: Address, amount: i128);
fn settle_claim(env, policy_id: u64, payout: i128) -> i128;

// views and operations
fn available_capacity(env) -> i128;
fn state(env) -> PoolState;
fn position(env, staker: Address) -> PositionView;   // shares, value, premium, lock
fn share_value(env) -> i128;
fn bump_position(env, staker: Address);             // permissionless TTL extension
fn set_policy_contract(env, policy: Address);       // admin, once
```

**Why the beneficiary is stored in the pool.** The obvious design has the pool read the payout recipient back from the Policy contract at settlement. On Soroban that call cannot happen. Settlement starts at `Policy.settle`, so the Policy contract is already on the call stack when the pool runs, and the host rejects any call back into it (2.9). The pool therefore records the beneficiary when it reserves capacity and pays that address at settlement. The Policy contract supplies only the payout amount, and the pool caps it at the reserved limit. Even a faulty Policy contract cannot redirect an existing reservation's payout, or pay more than its limit.

**Authorization.** The four policy-facing functions begin by requiring auth from the Policy contract registered at deployment and reject any other caller. Staker functions require the staker's own auth. `bump_position` is permissionless because it cannot move value.

### 4.3 Design decisions

**Custody-lock, never allowance.** `stake` moves the asset into the pool contract's own balance in the staker's signed transaction. From that ledger the pool owns the funds and needs no staker authorization to pay a claim. The alternative fails on its own terms: SEP-41 states that "an allowance is a spending limit, not a reservation. It does not lock any of `from`'s balance and may exceed it," and a staker could revoke it the moment a loss looked likely.

**Segregated pools, not one mutual.** A pool underwrites the policies written against it, so a staker chooses the risk they hold instead of inheriting an aggregate they cannot inspect.

**Fully collateralised, so the pool cannot fall short.** Coverage written never exceeds capital, and `unstake` refuses any withdrawal that would take capital below coverage written. Together these make a shortfall impossible rather than merely unlikely. If capital ≥ coverage before a claim, and the payout is at most that policy's limit, then after the claim `capital − payout ≥ coverage − limit`. Every remaining policy stays fully backed, whatever order claims arrive in.

**Reservation is atomic with issuance.** `reserve` executes inside the issuance transaction and fails if capacity is short. Over-writing coverage is prevented by the contract, not by monitoring.

**Lock period.** A stake cannot be withdrawn before `locked_until`. Combined with full collateralisation, this means capital committed behind a live policy stays until that policy settles or expires.

**Invariants, enforced on every transition:**

```
coverage_written <= total_capital
unstake leaves total_capital >= coverage_written
payout <= reservation.limit
```

### 4.4 Scheduled for milestone 1

The MVP covers the full happy path. Three refinements follow in milestone 1:

- **A notice period after the lock**, so a withdrawal is announced before it executes.
- **A claim freeze keyed to pending bad debt.** Withdrawals pause while the covered Blend reserve shows bad debt sitting on its backstop (9.1). The freeze is triggered by real bad debt in Blend rather than by anyone calling a function, so it cannot be cheaply griefed, and it is bounded by a deadline.
- **USDC pools.** The pool asset is already a constructor parameter.

Fractional collateralisation is not planned for this grant. If it is ever introduced, the pool would need a pro-rata shortfall rule rather than first-come settlement, which rewards whoever races the ledger.

---

## 5. Contract 2 — Policy

Parametric bad-debt cover on a supply position in a Blend reserve. Source: [`contracts/policy`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/policy).

### 5.1 Storage and interface

```rust
#[contracttype]
pub struct Terms {                 // set at deployment, applies to every policy
    pub admin: Address,
    pub pool: Address,
    pub adapter: Address,
    pub premium_bps: u32,          // premium as bps of the limit
    pub trigger_bps: u32,          // minimum b_rate decline that pays
    pub duration_ledgers: u32,
}

#[contracttype]
pub struct Policy {
    pub buyer: Address,
    pub beneficiary: Address,
    pub target: Address,           // Blend pool
    pub asset: Address,            // reserve asset supplied in that pool
    pub covered_amount: i128,      // size of the covered supply position
    pub limit: i128,               // maximum payout, reserved in the CoverPool
    pub premium: i128,
    pub baseline_rate: i128,       // b_rate observed at issuance
    pub start_ledger: u32,
    pub end_ledger: u32,
    pub status: Status,            // Active | Paid | Expired
    pub payout: i128,
}

fn issue(env, buyer, beneficiary, target, asset, covered_amount, limit) -> u64;
fn check(env, policy_id: u64) -> Check;    // live reading against the trigger
fn settle(env, policy_id: u64) -> i128;    // permissionless
fn expire(env, policy_id: u64);            // permissionless, after end_ledger
fn get(env, policy_id: u64) -> Policy;
```

### 5.2 The payout rule, and why the backstop is the deductible

```
decline = (baseline_rate − current_rate) / baseline_rate
payable = decline ≥ trigger_bps
payout  = min(covered_amount × decline, limit)
```

A supplier's claim on a Blend reserve is their bTokens × `b_rate`, so a fall in `b_rate` is exactly the supplier's proportional loss. The payout is therefore indemnity-shaped, not a binary lump sum: the vault receives what it actually lost, up to the limit it bought.

The backstop acts as the deductible without any parameter. Blend lowers `b_rate` only when it defaults debt that the backstop could not absorb (9.1). So this trigger fires only on losses that have already got past the backstop. Catalysis sits behind the backstop, never in place of it, and the backstop keeps doing the job it is good at. The trigger threshold then filters out socialized losses too small to be worth a claim.

### 5.3 Settlement is permissionless, and that is a property not a feature

Anyone may call `settle`, and the payout goes to the beneficiary recorded at reservation, whoever calls. There is no claims role, no adjudicator, and no code path by which a Catalysis-controlled key can withhold a valid payout or redirect one: the recipient is fixed in the pool (4.2), and the trigger condition is computed from Blend's own state in the same transaction. The trade-off is that every trigger must be **decidable from on-ledger state**, which is the constraint that shapes section 9.

### 5.4 Trigger types beyond the MVP

The MVP implements the Blend trigger described above. Scheduled next:

- **Pending bad debt as an early warning** (milestone 1). It drives the withdrawal freeze in 4.4 and never pays out on its own.
- **Share-value decline for DeFindex vaults** (milestone 2).
- **Oracle-derived triggers**, for positions that have no accounting signal (milestone 2, section 9.2).

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

Cover is written in the pool's own asset. An XLM pool covers XLM supply positions, and the largest single leg of the February loss was 61.25 million XLM borrowed out of the pool's XLM reserve. For cover denominated in a stablecoin, the pool asset is a constructor parameter, so a USDC pool is a redeploy rather than a code change. That pool carries Circle's freeze right, which we would disclose per policy.

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

The pool holds `total_capital` against `total_shares`, and a share is a claim on `total_capital / total_shares`. Settlement reduces capital and leaves every share count untouched. The loss therefore lands on all stakers pro rata, instantly, with no per-position work and nothing to realize later. This is the slashing guarantee, and it is the implemented code:

```rust
pub fn settle_claim(env: Env, policy_id: u64, payout: i128) -> i128 {
    require_policy_contract(&env);
    let cfg = config(&env);
    let res = reservation(&env, policy_id);          // limit + beneficiary
    if payout > res.limit {
        panic_with_error!(&env, Error::PayoutExceedsLimit);
    }

    let mut st = state(&env);
    st.total_capital -= payout;                      // every staker slashed pro rata
    st.coverage_written -= res.limit;
    st.claims_paid += payout;
    env.storage().persistent().remove(&DataKey::Reservation(policy_id));
    save_state(&env, &st);                           // pool state

    token::Client::new(&env, &cfg.asset)             // pool and beneficiary balances
        .transfer(&env.current_contract_address(), &res.beneficiary, &payout);
    payout
}
```

That is three state writes: the pool state, and the two balances the transfer touches. The count stays the same however many stakers there are.

This is the mechanism Blend itself uses — `default_liabilities` socializes a loss by moving `reserve.data.b_rate`, not by touching balances — and the one ERC-4626 uses. We rejected two alternatives:

- **Epoch lists** make a dormant staker walk every loss since they joined, so the cost of an interaction grows with the pool's history.
- **A loss-per-share accumulator with share burning** has a subtler flaw. Reducing one holder's share count changes the per-share value for everyone else, so each holder's realized loss starts depending on when the others happened to interact.

Unstaking pays `shares × total_capital / total_shares`. Staking mints `amount × total_shares / total_capital`. Both round down, in the pool's favour, so rounding dust can never make the pool insolvent. Because the share price is computed from internal accounting rather than the token balance, a direct transfer into the pool cannot inflate it.

**Premium does need an accumulator,** because it is claimed separately rather than folded into share value:

- On accrual, `premium_index` advances by `amount × 10¹⁸ / total_shares`.
- A staker's earned premium is `shares × (premium_index − premium_index_entry) / 10¹⁸`. It is settled into their own entry whenever their share count changes, so claiming writes only that one entry.

The integration test replays a 12% loss with two stakers at a 60/40 split. Each is slashed exactly 6% and keeps exactly their share of the premium.

---

## 8. Rent, TTL and liveness

Staker positions must survive a lock period during which the staker does nothing, and policies must survive to their end ledger. Both are persistent entries with independent TTLs (2.5).

**Today.** Every write extends the entry it touches, so any position or policy that is interacted with stays live. `bump_position` lets anyone extend a staker's entry without their signature.

**Milestone 2: a funded keeper.** A standing deduction from premium funds TTL maintenance. The position exists because a policy was sold, so the premium that policy generates keeps it alive. Stakers are not asked to fund rent on a position they cannot touch during the lock, and depositors in covered vaults are not charged for our storage at all.

TTL extension has no access control, so anyone may extend any entry. We will publish the keeper so third parties can run one.

**Why correctness does not depend on it.** The docs warn against relying on TTL extension for safety, and we do not. From protocol 23 an archived persistent entry included in a transaction's restore list is restored automatically when the invocation executes (2.5), so a claim against an archived position succeeds without a separate operation — it costs more in restore fees and footprint, but it does not fail. The keeper keeps claims cheap; a claim is correct whether or not it ran.

Rent pricing is itself a live network parameter rather than a constant — stellar-core's persistent rent-rate denominator changed materially at protocol 23 — so our fee model reads current values from the network rather than embedding them. Rent is an operating cost with no EVM analogue and appears as an explicit line in the delivery plan.

---

## 9. Triggers: what is decidable on-ledger

Section 5.3 requires every trigger to be decidable from on-ledger state. This section says how.

### 9.1 The payout trigger reads accounting, not prices

The loss event we most want to cover is the one where the oracle has already failed, so making the payout depend on an oracle in that moment would repeat the mistake one layer up. The payout trigger is state-derived.

**On Blend, the observable has to be chosen carefully, because the obvious one disappears at the moment of the event.** Bad debt is not a readable field: `bad_debt(user)` is a state-changing transition whose documented behaviour is that the backstop "will take over the debt," and if the backstop is itself unhealthy it "will default it's remaining debt." Pending bad debt therefore shows up as d-token liabilities sitting on the backstop address — but socialization, in `default_liabilities`, *removes* those liabilities and reduces `reserve.data.b_rate`. An adapter that watched backstop liabilities would see its signal go to zero exactly when the covered event occurred.

So the two states are separate triggers with different jobs:

- **`BadDebtPending`** (milestone 1) reads backstop liabilities and is an early warning: it can open monitoring and inform pricing, and it never pays out. It is also poke-dependent — debt reaches the backstop only when someone calls `bad_debt(user)`, and that call panics while a liquidation or bad-debt auction is open — so its absence proves nothing.
- **`BadDebtSocialized`** reads `b_rate` on the covered reserve against the baseline recorded at issuance and is the payout trigger — implemented. A `b_rate` decline beyond `threshold_bps` is the on-ledger signature of supplier loss, corroborated by Blend's `defaulted_debt` event.

```rust
// blend_adapter (implemented) — read-only, through Blend's public interface (2.8)
pub fn observe(env: Env, target: Address, asset: Address) -> i128 {
    BlendPoolClient::new(&env, &target).get_reserve(&asset).data.b_rate
}

// policy (implemented) — evaluated inside settle, in the same transaction
let current = AdapterClient::new(&env, &terms.adapter).observe(&p.target, &p.asset);
let decline_bps = (p.baseline_rate - current) * BPS / p.baseline_rate;
let payable = decline_bps >= terms.trigger_bps as i128;
let payout  = (p.covered_amount * (p.baseline_rate - current) / p.baseline_rate).min(p.limit);
```

The `Reserve`, `ReserveConfig` and `ReserveData` types in the repository mirror `blend-contracts-v2` field for field. That lets the same adapter read a real Blend pool as easily as the mock used in tests; milestone 1 points it at a Blend V2 pool on testnet.

Four properties of Blend's read surface shape the adapter:

- **Blend does not expose `get_backstop()`.** The pool's public interface is `get_config`, `get_admin`, `get_reserve_list`, `get_reserve`, `get_positions`, `get_reserve_emissions`, `get_user_emissions`, `get_auction`, and a contract cannot read another contract's storage (2.8). The backstop address is therefore recorded in the policy at issuance and verified off-chain against the pool's deployment.
- **`Positions` is keyed by reserve index, not asset.** `liabilities: Map<u32, i128>` with accessor `get_liabilities(reserve_index)`, so the index is resolved first via `get_reserve(asset).config.index` or `get_reserve_list()`.
- **These reads are point-in-time.** `Reserve::load()` updates to the current timestamp and auction data scales with elapsed ledgers, so nothing is cached across ledgers except the issuance baseline, which is stored on the policy deliberately.
- **`pool_data` on the backstop calls into the Comet AMM** for total supply and balances, so the Comet contract lands in the footprint of any read that uses it.

We call no state-changing Blend function. Coverage attaches without Blend granting us anything. For accuracy on a related point: Blend V2 bounds borrowing through `max_util` and a per-asset `supply_cap` rather than a per-asset borrow cap, so any risk-parameter scoring we publish names those fields.

**On DeFindex the shape differs and the read path is not free of side effects.** `fetch_total_managed_funds()` returns a per-asset idle-versus-invested breakdown; there is no `total_assets()` and no scalar share price, so the adapter computes NAV from that basket and the vault's `total_supply`. Critically, `get_asset_amounts_per_shares()` internally calls `fetch_total_managed_funds(e, true)`, and that flag — per DeFindex's own comment — "ensures that strategy reports are updated and new fees are locked." Calling it locks fees inside someone else's vault. **We therefore never call it from a permissionless entrypoint.** Even `fetch_total_managed_funds()` bumps the vault's instance TTL, so DeFindex evaluation is a read-write transaction whose footprint includes the vault and its strategy entries and scales with assets × strategies rather than being constant. That cost is disclosed here and priced into those policies rather than discovered by PaltaLabs later.

**DeFindex does not implement SEP-56.** SEP-0056 is a Draft (v0.1.2, November 2025) authored by OpenZeppelin and Sentinel specifying a 17-function single-asset `TokenizedVault` trait; DeFindex's `VaultTrait` is multi-asset by construction and implements none of `total_assets`, `convert_to_shares`, `convert_to_assets` or the `preview_*`/`max_*` family. An integration targets DeFindex's own interface. We track SEP-56 for the generic adapter and treat its Draft status as an integration risk rather than a foundation — OpenZeppelin's shipped `FungibleVault` already diverges from the draft's trait naming. SEP-40 is likewise a Draft (v0.1.0), and we treat it the same way.

### 9.2 Oracle-derived triggers, and where they are not available

*Milestone 2.* Price- and NAV-type triggers exist for cases with no equivalent accounting signal. Four conditions must all pass.

**Two independent feeds must agree.** We read Reflector's Stellar DEX feed and its external CEX/DEX feed and require divergence within `threshold_bps`. Both feeds' entries fit comfortably in one footprint — roughly 48 price records for two 2-hour windows, plus instance and code entries, against `txMaxFootprintEntries` of 400 — and an attacker cannot skip the check by omitting a feed, because touching an entry outside the declared footprint traps. Each feed quotes in its own `base()` with its own `decimals()`, so the adapter normalizes both before comparing, and each asset entry carries its own TTL via `expires(asset)`, which is checked before use.

**The window is built consumer-side and is visibly longer than the one that failed.** Reflector exposes no TWAP (2.6), so the adapter pulls `prices(asset, records)` and time-weights them itself. At the default 5-minute resolution a 2-hour window is twenty-four records, against a 24-hour retention ceiling. We default to 2 hours for deviation triggers: forensics of the February incident show oracle price entries five minutes apart, so a window measured in hours raises the cost of sustaining a manipulated price by orders of magnitude.

*On the window that failed:* Halborn describes the feed as a 15-minute VWAP, while Reflector's public Pulse feeds default to 5-minute resolution — which suggests the failing oracle may not have been the canonical Pulse DEX feed but a differently provisioned deployment. We do not resolve this from secondary sources; the adapter reads `resolution()` from the specific contract it is wired to rather than assuming either value.

**Depth, where it is readable, must clear a floor.** AMM reserves are contract state and can be read; the classic orderbook cannot be (2.7). Aquarius is effectively the only venue with volume — Soroswap shows $0 over both 24 hours and 30 days on DefiLlama — so in practice the depth floor applies where an Aquarius pool exists and nowhere else.

**Staleness is checked.** SEP-40's `PriceData` carries only `price` and `timestamp` — no confidence interval, no round id — so timestamp age against the feed's `resolution()` is the only staleness signal available, and an observation older than a configured bound is not actionable.

**Where the gates are unavailable, no price trigger is offered.** This is the honest and load-bearing case, not a corner case. A tokenized treasury with sub-$1 hourly volume on its only venue will have no Aquarius pool, and Reflector feeds carry an explicit per-asset list, so an asset the external feed does not quote returns `None` rather than a comparison price. Both oracle-side gates then have nothing to work with. The rule is **fail-closed**: where the external feed does not quote the asset, no price- or NAV-type trigger is available and the policy is written on accounting triggers only. Confirming per-asset coverage against `assets()` on the external feed is a milestone-1 task, and it is why 9.1 is the primary path rather than a fallback.

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
| Payout redirected, by us or anyone | Beneficiary fixed in the pool at reservation; `settle` takes no recipient; no claims role exists (4.2, 5.3) | Implemented |
| Payout larger than the cover bought | Pool caps every payout at the reserved limit (4.3) | Implemented |
| Unauthorized caller reserves, releases or settles | All four policy-facing pool functions require auth from the registered Policy contract (4.2) | Implemented |
| Pool unable to pay a valid claim | Full collateralisation; capital backing coverage cannot be unstaked (4.3) | Implemented |
| Capacity over-writing | Reservation atomic with issuance; `coverage_written <= total_capital` (4.3) | Implemented |
| Issuer freezes or claws back pool capital | Pool holds native XLM, which has no issuer (6.2) | Implemented |
| Allowance revoked when a loss looks likely | Custody-lock staking; no `transfer_from` path (4.3) | Implemented |
| Settlement cost grows with stakers | Loss absorbed by share value; three writes, constant in staker count (7.2) | Implemented |
| Share price inflated by a direct transfer | Share price computed from internal accounting, not token balance (7.2) | Implemented |
| Rounding drift | Mint and redeem round down, in the pool's favour (7.2) | Implemented |
| Oracle manipulation to suppress a valid payout | Payout trigger reads Blend's `b_rate`, not a price (9.1) | Implemented |
| Staker flight ahead of a known loss | Lock period today; notice period and a claim freeze keyed to pending bad debt (4.4) | Milestone 1 |
| Oracle manipulation to force a spurious payout | Cross-feed divergence gate, 2-hour time weighting, staleness bound; fail-closed where gates are unavailable (9.2) | Milestone 2 |
| Position archived when a claim lands | Protocol-23 automatic restore; interactions extend TTL today, keeper in milestone 2 (8) | Partial |
| Our adapter used to force side effects in a covered vault | Side-effecting vendor functions kept out of permissionless paths (9.1) | Milestone 2 |
| Cross-contract reentrancy | Blocked by the host (2.9); the pool never calls back into Policy (4.2) | Implemented |
| Admin key compromise | 10.3 | Before mainnet |

### 10.2 Testing

Implemented, and run in CI on every push alongside `cargo fmt`, `cargo clippy -D warnings` and a `wasm32v1-none` release build:

- **Eleven CoverPool unit tests.** Share minting, the lock period, capital backing coverage being unable to leave, capacity limits, pro-rata premium, pro-rata slashing on payout, a staker joining after a loss not being charged for it, release, the payout cap, and caller authorization.
- **Three end-to-end tests across all four contracts.**
  1. A bad-debt event: `b_rate` falls 12%, a third party settles, the vault is paid exactly its loss, both stakers are slashed exactly 6%, and they unstake with their premium.
  2. A quiet term: the policy expires, capacity is released, and stakers exit with capital plus premium.
  3. A 1% wobble below the trigger, which does not pay.

Planned for milestone 1:

- property tests on the pool invariants and share arithmetic across randomized sequences
- the same scenario against a real Blend V2 pool on testnet
- a replay of the February price path against mocked Reflector feeds, for the milestone-2 oracle triggers

### 10.3 Admin authority and its limits

In the MVP the admin can do three things:

- register the Policy contract, once
- upgrade the contracts
- set `b_rate` on the mock Blend pool, which is a demo harness and not part of the product

Underwriting terms — premium rate, trigger threshold, duration — are fixed when the Policy contract is deployed.

No admin function can alter an issued policy, redirect or block a settlement, withdraw staked capital, or change share value other than through a settled claim. The upgrade path is the admin's one broad power, so before mainnet it moves behind a multisig with a timelock, both published on deployment.

### 10.4 Audit and licensing

Sigma Prime led our Ethereum audits (Core, March 2026; Coverage, April 2026; combined, May 2026), with Certora on the covered vault (March 2026). All four reports are public and linked in section 14.

For the Soroban contracts we engage a Soroban-experienced auditor, with scope agreed before implementation completes and remediation closed before mainnet. SCF audit credits cover the cost, and no audit expense appears in the budget. The Soroban contracts are published under Apache-2.0.

---

## 11. Delivery

Three milestones. The final one is mainnet launch. Every deliverable has a binary completion test.

| Milestone | Scope | Target |
|---|---|---|
| 1 | CoverPool and Policy integrated against a real Blend V2 pool on testnet; pending-bad-debt signal with withdrawal notice period and claim freeze; USDC pool alongside XLM; property tests | 31 Jan 2027 |
| 2 | Oracle trigger module with cross-feed divergence gate and consumer-side time weighting; DeFindex adapter; keeper-funded TTL maintenance; risk-parameter scoring against live Blend pools; STRIDE threat model and monitoring plan; audit engaged with scope frozen | 28 Feb 2027 |
| 3 | Audit remediation; multisig and timelock on admin; mainnet deployment; one live covered vault with staked capacity; monitoring, SDK and integration docs | 31 Mar 2027 |

Build starts in the first week of January 2027 and completes within the first quarter.

The published #46 calendar runs:

- submission closes 8 November 2026
- panel review 11 to 18 November
- community vote 23 November to 3 December

Notification, information collection and KYB follow the vote, so tranche #0 may arrive while milestone 1 is under way; we start on our own balance sheet rather than wait for it. Each tranche completion form falls well inside the 90-day window that runs from receipt of the preceding payment.

**The critical path is the audit, not the engineering.** Mainnet by 31 March needs an audit slot, the audit itself and remediation to fit inside March. The audit is therefore engaged during milestone 2 with scope frozen at the end of February. Milestone 3 adds no new contract functionality — only remediation, deployment and the first live vault.

**Milestone 1 starts partly delivered.** The Ethereum implementation of this system is shipped and audited, so the Soroban work is a reimplementation against a known specification rather than a design exercise. And the full happy path is built before this submission: staking, cover issuance, the Blend trigger, pro-rata slashing, premium distribution and unstaking, with tests, CI and a testnet demo. Contract IDs are published with the submission.

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
