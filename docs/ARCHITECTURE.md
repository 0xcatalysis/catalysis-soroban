# Catalysis on Soroban — Technical Architecture

**Covered vaults for Stellar: staked XLM insurance that pays depositor shortfalls at withdrawal**

Version 1.4 · 25 September 2026 · Prepared for Stellar Community Fund #46, Build Award (Open Track)
Authors: Abhishek Kumar, Dhruv Bodani · Catalyst Labs LLC FZ

---

## 0. Scope

This is the on-ledger design of a coverage protocol for Stellar: the contracts, their storage and interfaces, the accounting, how a shortfall is determined and paid, the security model, and the milestones to mainnet. It is what the SCF #46 build executes against, and it describes only what the code does today or what a named milestone adds.

**Implementation status.** The covered-vault happy path in sections 3 to 7 is implemented, tested and deployed to Stellar testnet: CoverPool, CoveredVault, a mock strategy standing in for a Blend market (test and demo only), 83 passing tests including the end-to-end demo scenario, and a script that has run that scenario on testnet. Contract IDs are in the repository README and in `deployments/testnet.json`.

**Where everything lives.** Source, tests, the testnet demo script and deployment IDs are at https://github.com/0xcatalysis/catalysis-soroban. Alongside this document the repository holds the build plan and task log (`docs/PLAN.md`), the design decision log with the reason for every decision (`docs/DECISIONS.md`), and the background research this document draws on, including the February 2026 incident forensics, the full list of Stellar constraints, the ledger evidence on issuer flags, and the Blend, DeFindex and oracle read surfaces (`docs/BACKGROUND.md`). There is no user interface in this grant's scope: the product is contracts, a CLI demo and an SDK (9), so there are no UI/UX prototypes.

**AI disclosure.** The contract code, tests, scripts and documentation in the repository were written with AI assistance (Claude Code) and reviewed by the team, who own every design decision; the decision log records each one. Research synthesis and drafting for this document were AI-assisted in the same way. The protocol design, the architecture and all technical claims are the team's own, and were verified against primary sources on 25 September 2026.

**Reading order for a reviewer with limited time:** section 2 (why this is Stellar-specific), section 4 (the staked pool and its slashing guarantee), section 5 (the covered vault and payout at withdrawal), section 9 (milestones).

---

## 1. The problem, and what exists today

Stellar has roughly $3.4B to $4.0B of tokenized real-world assets on-ledger (Stellar's own Dune dashboard reported $3.996B on 29 August 2026; rwa.xyz reports about $3.45B). Against that, the chain's entire DeFi TVL is about $260M, and Blend, its primary lending market, holds $164.4M with $43.7M in active loans (DefiLlama, 25 September 2026). The overwhelming majority of tokenized value on Stellar does not work as collateral anywhere. The constraint is not issuance and it is not contract quality. It is that nobody owns credit-risk curation, nobody is paid for it, and when curation fails there is no capital positioned to absorb the loss.

**What that looks like when it fails.** On 22 February 2026 an attacker manipulated the price of USTRY, a tokenized US Treasury instrument, in a Blend V2 pool operated by the YieldBlox DAO, and borrowed about 1.0M USDC and 61.25M XLM against collateral worth a small fraction of its marked value: roughly $10.2M to $11.0M depending on how the XLM leg is priced. Three properties of that event shape this design. The contracts were not broken: Blend V2's core had been through Certora formal verification, and the loss came from pool configuration, the choice of oracle and the parameters around it, which is exactly the risk surface nothing on Stellar prices. Resolution was discretionary: Script3 compensated depositors, and recovery depended on Tier-1 validators freezing about 48M XLM. And under Blend's documented design, residual bad debt is socialized among that pool's lenders. The full forensics, with sources and the ranges where sources disagree, are in `docs/BACKGROUND.md` part A.

**Why Blend's backstop cannot be the answer.** Blend's only bad-debt absorption mechanism is its backstop, denominated in BLND:USDC 80:20 liquidity pool shares. That is the governance token of the protocol being backstopped (BLND is down about 95% from its 2024 high), held inside an AMM pool whose bugs are the backstop's bugs, behind a 17-day withdrawal queue. A first-loss buffer bounded by the depth of its own collateral is not risk transfer. It is a deductible.

**What Catalysis adds.** A second-loss layer: exogenous underwriting capital committed to a named vault under contract, with the payout computed on-ledger from the vault's own accounting and paid inside the depositor's withdrawal. Capital that is not the covered protocol's own token. No claims committee, no governance vote, no discretionary compensation, no validator coordination. We have built this once, on Ethereum, audited and live; what follows is a different system in the ways Stellar requires.

### 1.1 Existing solutions, and how this differs

Nothing on Stellar or Soroban does this today. The two mechanisms that absorbed the February loss are the ones described above: Blend's backstop, which is first-loss capital in the protocol's own token (1.2), and discretionary compensation by the pool's operator (1.1). We found no on-ledger cover protocol on Stellar, and no product that lets an XLM holder earn premium for underwriting a named vault.

Elsewhere, DeFi cover exists in three shapes, and the design here differs from each on the same axis: who decides a claim, and what the payout is measured against.

| | Who decides a claim | What backs it | How loss is measured |
|---|---|---|---|
| Nexus Mutual (Ethereum) | Members vote on each claim by staking the mutual's token ([docs](https://docs.nexusmutual.io/)) | The mutual's pooled capital, priced by staked NXM | Claimant's evidence, assessed by vote |
| Sherlock (Ethereum) | A claims committee, with escalation to UMA's optimistic oracle ([docs](https://docs.sherlock.xyz/)) | USDC staking pools per covered protocol | Off-chain determination of exploit loss |
| Blend backstop (Stellar) | Nobody: bad debt is auctioned against the backstop, then socialized (1.2) | BLND:USDC LP shares, the protocol's own token | Blend's own accounting |
| Catalysis on Ethereum | Nobody: the vault wrapper files the claim inside the depositor's withdrawal | Restaked ETH via EigenLayer and Symbiotic | The vault's accounting: principal against share value |
| **This design** | **Nobody: the vault claims inside the depositor's withdrawal (5.2)** | **Native XLM staked in a segregated pool (4, 6)** | **The vault's accounting (5.2)** |

The first two put a human decision between the loss and the payout, which is where cover protocols historically disappoint their buyers, and their capital is either the cover protocol's own token or a stablecoin an issuer can freeze. The Blend backstop has no decision but is the covered protocol's own token and is first-loss, so it is a deductible, not risk transfer. Catalysis on Ethereum removed the decision; it relied on restaking for capital, which Stellar does not have (2.2).

What this design adds over each: a payout with no decision-maker, measured by the covered vault's own books at the moment the depositor leaves; capacity in the network's base asset, which no issuer can freeze and which is not the token of any covered protocol; and full collateralisation enforced continuously rather than checked once (4.3). The value to each party is concrete. A depositor gets principal protection with no claim to file. A staker earns premium on XLM for underwriting a vault they chose. A curator can offer a covered vault on Blend without building any of this.

---

## 2. The constraints that shape this design

Each item is a property of Stellar or Soroban that changed a decision in the shipped code. The numbering is kept from the full list in `docs/BACKGROUND.md` part B, which also covers the oracle and DEX constraints that matter for work after mainnet.

**2.1 Issuer authorization flags reach inside contract balances.** An issuer's freeze and clawback rights apply to a balance held by a Soroban contract, not only to classic trustlines. A contract address is not a shelter. Capital an issuer can freeze is not capacity, and this has no Ethereum analogue. Section 6 is the response.

**2.2 There is no restaking, no shared security, and no slashable external capital.** On Ethereum our capacity comes from existing restaked ETH and someone else's slashing rail. Stellar has neither, so capacity is raised, committed, priced and drawn natively (4). This is the largest piece of new engineering in this build.

**2.3 Footprint limits bound the shape of loss accounting.** Per-transaction write and footprint limits are network settings set by validator vote, and at any value they mean settlement must never iterate over staker positions. Section 7 makes a claim cost the same number of writes whether the pool has ten stakers or a thousand.

**2.5 Persistent state archives.** Every staker position, depositor position and vault cover is a persistent entry with its own TTL. TTL extension has no access control, so a keeper can maintain positions without their owner's signature; and from protocol 23, archived entries in a transaction's restore list are restored automatically when the invocation executes, so a withdrawal against an archived position succeeds without a separate operation. Every write extends the entry it touches, and `bump_position` on both contracts lets anyone extend a position; a premium-funded keeper is milestone 2 (9).

**2.9 Cross-contract reentrancy is blocked by the host.** Every external call defaults to `ContractReentryMode::Prohibited`, and detection scans the whole context stack, so indirect reentry is caught too. The design therefore has no callbacks: the vault calls the pool and the strategies, they call only the token, and nothing calls back into the vault (3, 4.2).

**2.10 Upgrades replace Wasm and do not re-run constructors.** `update_current_contract` preserves address and storage, and CAP-0058 states that a constructor is not re-run on code update. The MVP has an admin-only `upgrade` on each contract and no state migration; an explicit, idempotent, version-gated `migrate()` arrives with the multisig in milestone 3.

**2.11 Network liquidity is thin and concentrated in a single venue.** Aquarius carries about 98.7% of DefiLlama-tracked Stellar DEX volume. No part of this system may assume it can liquidate collateral on-ledger at size: claims settle in the asset the pool already holds, and we never sell collateral into a Stellar venue to fund a payout.

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

**Blend V2 strategy — milestone 1.** Supplies XLM to a Blend pool through `submit` and values its position as `b_tokens × b_rate`, read through Blend's public `get_reserve` and `get_positions`. Blend socializes bad debt by lowering `b_rate` (`docs/BACKGROUND.md` part E), so a socialized loss shows up in `total_value()` and therefore in every depositor's shortfall, with no trigger. The `Reserve` types needed for that read are already in `contracts/common`, mirrored field for field from `blend-contracts-v2`.

### 5.5 What the vault does not do in the MVP

The Ethereum wrapper has controls the MVP deliberately omits: streaming premium charged to depositors, a per-depositor loss cap, a curator review gate between successive claims, a deposit cap, and an emergency exit that waives insurance. Section 8.4 lists these with the milestone that adds each.

---

## 6. Collateral: why the pool holds native XLM

Constraint 2.1 says an issuer's freeze and clawback rights reach balances held by our pool contract, and a contract cannot read those rights. We checked the ledger before choosing the pool asset, and the finding decided it.

**What we found.** We queried issuer authorization flags via Horizon for every Stellar asset we could attribute to a genuine issuer by `home_domain` and `stellar.toml`; the dated snapshot, the queries and a script that re-runs them are in `data/issuer-flags/` in the repository. Every issued asset we could verify is either freezable by its issuer today, or could be made freezable with a single signature: no verified issuer has set `AUTH_IMMUTABLE`. USDC and EURC (Circle) are freezable today. USDY (Ondo), BENJI (Franklin Templeton) and YLDS (Figure), three of the largest RWAs with attributable issuers, can be both frozen and clawed back. The table and the method are in `docs/BACKGROUND.md` part C.

**The choice: native XLM.**

Native XLM has no issuer. It has no authorization flags, no freeze and no clawback, so no party can touch capital staked in the pool. It is also the deepest asset on the network, and it needs no trustline. That makes it the one asset on Stellar where the problem in 6.1 does not arise, and it is what the pool holds.

The trade-off is price correlation, and we state it plainly. XLM's price tends to fall when the whole Stellar ecosystem is under stress, so the pool's capacity measured in dollars shrinks in exactly those conditions. This is a weaker form of the wrong-way risk we criticize in Blend's backstop (1.2), and the difference is worth being precise about. BLND is correlated with the one protocol it backstops, and it sits inside an AMM pool whose bugs are the backstop's bugs. XLM is the network's base asset, has no AMM dependency, and is not the token of any covered protocol.

Cover is written in the pool's own asset. An XLM pool covers an XLM vault, and the largest single leg of the February loss was 61.25 million XLM borrowed out of the pool's XLM reserve. For cover denominated in a stablecoin, the pool asset is a constructor parameter, so a USDC pool is a redeploy rather than a code change. That pool carries Circle's freeze right, which we would disclose per vault.

Adding issued assets as capacity needs machinery the MVP leaves out: an attested flag registry, a live `authorized()` check in the settlement path, and a rule that the pool's balance entry never closes. It is specified in `docs/BACKGROUND.md` part C and follows the grant.

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

## 8. Security

### 8.1 Testing

83 tests, run by `scripts/check.sh` together with `cargo fmt`, `cargo clippy -D warnings` and the `stellar contract build` of every contract; the same steps are in the GitHub Actions workflow.

- **34 CoverPool unit tests.** Share minting, the lock boundary, capital backing cover being unable to leave, exact capacity and one over, premium split by shares and not retroactive to late stakers, premium kept by stakers who leave, premium rounding and dust conservation, full and partial claims, three sequential partial claims against a 60/40 pair, two vaults sharing one pool's capacity, a staker leaving between claims, share price after an odd slash, one-unit stakes, the drained pool, and a scripted sequence that checks every invariant in 4.3 after each step.
- **39 CoveredVault unit tests.** Share minting at par and at a lower NAV, NAV across idle and two strategies, the demo scenario step by step, withdrawals with no loss, a partial loss, a total loss and a profit, partial withdrawals of different sizes sharing a loss pro rata, five depositors through a loss, depositors at different entry prices, a late depositor through two losses in either order, the same depositor depositing twice, interleaved partial and full exits, cover exhaustion leaving state unchanged, stroop-level rounding, zero-share and zero-payout rejection, and a scripted life of the vault that checks the books after every step.
- **8 MockStrategy unit tests.** Value, withdrawal, the demo's 20% loss, total loss, bounds and authorization.
- **2 end-to-end tests** across all three contracts: the worked example in 5.3 with every number asserted and every balance reconciled, and the quiet path.

The scenario tests were derived from the Ethereum Catalysis suites (the covered vault wrapper, claim manager, cover pool and premium manager), recomputed for the MVP's simpler rules. The testnet script has run the worked example end to end against the deployed contracts.

Planned for milestone 1:

- property tests on the pool and vault invariants across randomized sequences
- the same scenario against a real Blend V2 pool on testnet through the Blend strategy

### 8.2 Admin authority and its limits

The pool admin can whitelist vaults and upgrade the pool. It cannot alter a vault's cover, redirect or block a claim, withdraw staked capital, or change share value other than through a claim.

The vault admin is the curator. The curator can whitelist strategies, allocate idle deposits to them, buy and close cover, and upgrade the vault. The curator cannot touch a depositor's principal or shares, block a withdrawal, or take deposits out of the vault other than into a whitelisted strategy. Allocation is the curation role, and depositors trust the curator's judgement about strategies, exactly as depositors in a Morpho vault do; milestone 2 adds per-strategy allocation limits.

The mock strategy's admin can simulate a loss. That is a demo harness and not part of the product.

Underwriting terms — the premium rate and the lock — are fixed when the pool is deployed. The upgrade path is each admin's one broad power, so before mainnet it moves behind a multisig with a timelock, both published on deployment.

### 8.3 Audit and licensing

Sigma Prime led our Ethereum audits (Core, March 2026; Coverage, April 2026; combined, May 2026), with Certora on the covered vault (March 2026). All four reports are public at https://docs.catalysis.network/docs/audits/.

For the Soroban contracts we engage a Soroban-experienced auditor, with scope agreed before implementation completes and remediation closed before mainnet. SCF audit credits cover the cost, and no audit expense appears in the budget. The Soroban contracts are published under Apache-2.0.

### 8.4 Known MVP limitations

Each is a deliberate simplification of the Ethereum design, documented rather than fixed in the MVP. The full list with the Ethereum mechanism each corresponds to is in `docs/BACKGROUND.md` part F.

- **First-come compensation if cover is too small**, as on Ethereum. A withdrawal whose shortfall exceeds remaining cover fails outright; the MVP sizes cover above the maximum loss. Milestone 1 records shortfalls when a loss is reported and shares remaining cover pro rata.
- **Stakers can exit excess capital ahead of an unclaimed loss.** Capital backing cover cannot leave, so payouts are always funded. Milestone 1 adds loss reporting.
- **No cover expiry, no streaming premium, no fee split, no curator review gate between claims, deductible fixed at zero, no per-depositor loss cap.** All Ethereum features; milestone 1.
- **Share inflation by donation is only partly mitigated.** Zero-share mints are rejected; a virtual share offset is milestone 1.
- **A drained vault or pool rejects new money** rather than recovering. Withdrawals drain strategies in whitelist order, not pro rata. Shares are non-transferable. The admin can upgrade the contracts until the multisig and timelock in milestone 3.

---

## 9. Delivery

Three milestones. The final one is mainnet launch. This is a technical roadmap: every deliverable is code the team ships and runs, and every completion test in the third column is something the team can make true without any other party's participation. Blend and Reflector are permissionless, so integrating with them needs nobody's consent.

| Milestone | Scope | Done when | Target |
|---|---|---|---|
| 1 — Real yield, real losses | **Blend V2 strategy** on testnet: supplies and withdraws through Blend's `submit`, values its position through `b_rate` (5.4). **Loss reporting** at strategy level: the vault reports a loss to the pool when it happens, the pool records the shortfall, and remaining cover is shared pro rata rather than first-come (4.4, 8.5). **Staker notice period** after the lock. **Cover expiry and streaming premium** accrued per ledger from vault yield instead of paid upfront by the curator. **Property tests** on the pool and vault invariants across randomized sequences. | The demo scenario runs on testnet against a Blend V2 pool, with the loss coming from a real `b_rate` decline in that pool rather than a mock. An integration test and a testnet run show a reported loss slashing stakers before any depositor withdraws. A test shows two depositors whose combined shortfall exceeds remaining cover each paid the same fraction. Tests show premium accruing per ledger from yield and cover ending at its expiry ledger. Property tests run in `scripts/check.sh` and CI. | 31 Jan 2027 |
| 2 — Curation and operations | **Multiple Blend markets** per vault with curator **allocation limits** enforced on-chain (8.2). **Keeper** for TTL maintenance, funded from premium and open-sourced so anyone can run it (2.5). **USDC vault and pool** alongside XLM, with the issuer freeze right disclosed (6.2). **STRIDE threat model and on-chain monitoring plan**, published in this repository. **Contracts feature-frozen and tagged for audit.** | A vault on testnet holds positions in two Blend markets under limits the contract enforces, with a test showing an over-limit allocation rejected. The keeper is in this repository and has carried every position through a full TTL cycle on testnet. The demo runs end to end on a USDC vault and pool on testnet. The threat model and monitoring plan are committed. A tagged release marks the audit scope. | 28 Feb 2027 |
| 3 — Mainnet | **Audit remediation** with the report published. **Multisig and timelock** on every admin function (8.2), with `migrate()` for future upgrades (2.10). **Mainnet deployment.** **A covered vault live on mainnet**, operated by the team: pool, vault and Blend strategy deployed, stake in the pool, cover open, and the vault allocated to a Blend market. **SDK and integration docs** for curators and stakers. | All audit findings closed and the report linked from this document. Mainnet contract IDs published, every admin held by the multisig behind the timelock, verified from the ledger. The team has staked, bought cover, deposited, allocated to a Blend market and withdrawn on mainnet, with the transactions linked. The SDK is published with docs and the demo script uses it. | 31 Mar 2027 |

Oracle-derived triggers and a DeFindex strategy (`docs/BACKGROUND.md` part E) follow mainnet and are not in this grant's scope. They are specified there because they shape the strategy interface, and because a reviewer should see what the accounting-only design deliberately excludes.

Build starts in the first week of January 2027 and completes within the first quarter.

The published #46 calendar runs:

- submission closes 8 November 2026
- panel review 11 to 18 November
- community vote 23 November to 3 December

Notification, information collection and KYB follow the vote, so tranche #0 may arrive while milestone 1 is under way; we start on our own balance sheet rather than wait for it. Each tranche completion form falls well inside the 90-day window that runs from receipt of the preceding payment.

**Budget.** The budget is in the submission form, sized to two engineers over the three milestones. It carries ledger rent as its own line and no audit cost, which SCF credits cover (8.3).

**The critical path is the audit, not the engineering.** Mainnet by 31 March needs an audit slot, the audit itself and remediation to fit inside March. The audit is therefore engaged during milestone 2 with scope frozen at the end of February. Milestone 3 adds no new contract functionality — only remediation, deployment and the first live vault.

**Milestone 1 starts partly delivered.** The Ethereum implementation of this system is shipped and audited, so the Soroban work is a reimplementation against a known specification rather than a design exercise. And the full happy path is built before this submission: staking, cover purchase, deposits and curator allocation, a strategy loss, payout at withdrawal with pro-rata slashing, premium distribution and unstaking, with 83 tests, a local and CI check, and a testnet demo whose contract IDs are in the README.

---

## 10. On-chain growth, and how it is measured

The growth this unlocks is lending deposits. Every XLM in a covered vault is XLM supplied to a Blend market, and every XLM staked in a pool is XLM locked in a Soroban contract earning premium. Section 1 gives the gap: the chain has $3.4B to $4.0B of tokenized value and $164M in its primary lending market, and the reason is that nobody is paid to absorb a curation failure. A covered vault is the instrument that lets a depositor who would not accept the February outcome supply to Blend anyway, and lets an XLM holder be paid for standing behind that.

Every metric below is contract state or an event, readable by anyone from the ledger, so growth is measured without trusting us.

| Metric | Where it is read |
|---|---|
| XLM staked as cover capacity | `CoverPool.state().total_capital`, per pool |
| Cover written | `CoverPool.state().coverage_written`; `CoverOpened` events |
| XLM in covered vaults | `CoveredVault.nav()`, per vault; `Deposited` and `Withdrawn` events |
| Of which supplied to Blend | each strategy's `total_value()` |
| Depositors and stakers | position entries, from `Deposited` and `Staked` events |
| Premium paid to stakers | `CoverPool.state().premiums_paid` |
| Shortfalls paid to depositors | `CoverPool.state().claims_paid`; `Slashed` events, one per claim |
| Number of covered vaults and markets | `add_vault` and `add_strategy` events |

The monitoring plan in milestone 2 (9) publishes a dashboard over exactly these fields. The mainnet vault in milestone 3 is the first entry in it. What counts as success is the four balances that a covered vault moves on-ledger: capital staked, cover written, deposits covered, and premium paid, all in XLM, all from contract state.

---

## 11. Team

Two co-founders, both protocol engineers.

**Abhishek Kumar — CEO.** Ex-Product at Obol (DV Labs), protocol engineering at Nethermind, previously Amazon and Atlassian. Ethereum Proof-of-Stake, staking infrastructure, DeFi.

**Dhruv Bodani — CTO.** Ex-Tech Lead for L2s at Obol (DV Labs), Senior Engineer at Offchain Labs (Arbitrum), protocol engineering at Nethermind, previously Visa and Atlassian. Distributed and P2P systems, Ethereum PoS, DeFi.

We designed, audited and shipped Catalysis to Ethereum mainnet — vault-native risk coverage, with coverage operating over a Tier-1 curator's vault on Morpho and underwriting capacity sourced from EigenLayer and Symbiotic. Audited by Sigma Prime and Certora. Both founders co-authored *Economic Security of Multiple Shared Security Protocols* (Marble 2025, Imperial College Brevan Howard Centre for Financial Analysis), on the economic security of restaking across shared security providers.

---
