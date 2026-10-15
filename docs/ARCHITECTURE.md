# Catalysis on Soroban — Technical Architecture

**Covered Vaults for Stellar Markets.** Staked XLM insurance that pays a vault depositor's shortfall inside the withdrawal.

Version 1.5 · 28 September 2026 · Stellar Community Fund #46, Build Award (Open Track)
Authors: Abhishek Kumar, Dhruv Bodani · Catalyst Labs LLC FZ

---

## 0. Scope and status

This document specifies the contracts, their storage and interfaces, the accounting, the security model and the milestones to mainnet. It describes what the code does today. Milestone work is marked as such.

**Status.** CoverPool and CoveredVault are written in Rust, covered by 83 passing tests, and deployed to Stellar testnet. The scenario in 5.3 has run end to end against the deployed contracts. A mock strategy stands in for a Blend market; it is for tests and the demo only, and milestone 1 replaces it.

| | |
|---|---|
| CoverPool, testnet | [`CB3SK2KHIBOKV3WHBMXMJOYQJDSGCXJ6GGFZZN5UZM63K25B5GTBNVIO`](https://stellar.expert/explorer/testnet/contract/CB3SK2KHIBOKV3WHBMXMJOYQJDSGCXJ6GGFZZN5UZM63K25B5GTBNVIO) |
| CoveredVault, testnet | [`CDMDOL3XKEHHUOCX7M7QRLCBH6AULKHGHGTKIQXOW6HSC7GJKLBAU27A`](https://stellar.expert/explorer/testnet/contract/CDMDOL3XKEHHUOCX7M7QRLCBH6AULKHGHGTKIQXOW6HSC7GJKLBAU27A) |
| Source, tests, demo script | https://github.com/0xcatalysis/catalysis-soroban |
| Licence | Apache-2.0 |

The product is contracts, a CLI demo and an SDK (9). There is no user interface in scope.

**AI disclosure.** The contract code, tests, scripts and documentation were written with AI assistance (Claude Code) and reviewed by the team, who own every design decision. Research synthesis and drafting for this document were AI-assisted in the same way. The protocol design, the architecture and all technical claims are the team's own and were verified against primary sources.

---

## 1. The problem

Stellar holds $3.4B to $4.0B of tokenized real-world assets (Stellar's Dune dashboard, 29 August 2026; rwa.xyz). Its entire DeFi TVL is about $260M, and Blend, the primary lending market, holds $164.4M with $43.7M in active loans (DefiLlama, 25 September 2026). Most tokenized value on Stellar does not work as collateral anywhere. Nobody owns credit-risk curation, nobody is paid for it, and when curation fails no capital is positioned to absorb the loss.

**22 February 2026.** An attacker manipulated the price of USTRY in a Blend V2 pool operated by the YieldBlox DAO and borrowed about 1.0M USDC and 61.25M XLM against it: $10.2M to $11.0M, depending on how the XLM leg is priced.

- The contracts were not broken. Blend V2's core had passed Certora formal verification. The loss came from pool configuration: the choice of oracle and the parameters around it.
- Resolution was discretionary. Script3 compensated depositors, and Tier-1 validators froze about 48M XLM.
- Under Blend's design, residual bad debt is socialized among the pool's lenders.

**Blend's backstop** is the only bad-debt mechanism on Stellar. It is denominated in BLND:USDC 80:20 LP shares: the governance token of the protocol it backstops, inside an AMM pool, behind a 17-day withdrawal queue. It is first-loss capital bounded by the depth of its own collateral.

**What Catalysis adds.** Second-loss capital that is not the covered protocol's own token, committed to a named vault, with the payout computed from the vault's own accounting and paid inside the depositor's withdrawal. No claims committee, no vote, no discretionary compensation.

### 1.1 Existing solutions

We found no live on-ledger protocol on Stellar that pays a vault depositor's credit loss from staked capital, and none that lets an XLM holder earn premium for underwriting a named vault.

| | Who decides a claim | What backs it | How loss is measured |
|---|---|---|---|
| Nexus Mutual (Ethereum) | Members vote, staking the mutual's token ([docs](https://docs.nexusmutual.io/)) | The mutual's pooled capital | Claimant's evidence, assessed by vote |
| Blend backstop (Stellar) | Nobody: bad debt is auctioned against the backstop, then socialized | BLND:USDC LP shares | Blend's accounting |
| Catalysis on Ethereum | Nobody: the vault wrapper filed the claim inside the withdrawal | Restaked ETH via EigenLayer and Symbiotic | The vault's accounting |
| **This design** | **Nobody: the vault claims inside the withdrawal (5.2)** | **Native XLM staked in a segregated pool (4, 6)** | **The vault's accounting (5.2)** |

This design differs from each on three points: no decision-maker between the loss and the payout; capacity in the network's base asset, which no issuer can freeze and which is not the token of any covered protocol; and full collateralisation enforced on every state change (4.3).

---

## 2. Stellar constraints that shaped the design

Each is a property of Stellar or Soroban that changed a decision in the shipped code.

**2.1 Issuer flags reach inside contract balances.** An issuer's freeze and clawback rights apply to balances held by a Soroban contract. Capital an issuer can freeze is not capacity. The pool therefore holds native XLM (6).

**2.2 No restaking and no slashable external capital.** On Ethereum our capacity came from restaked ETH and an existing slashing rail. Stellar has neither, so capacity is raised, priced and slashed natively in the CoverPool (4).

**2.3 Footprint limits.** Per-transaction write and footprint limits are network settings. Settlement must never iterate over staker positions, so a claim costs the same number of writes at any pool size (7).

**2.4 Persistent entries archive.** Every position and every vault's cover is a persistent entry with its own TTL. Every write extends the entry it touches, `bump_position` lets anyone extend a position, and from protocol 23 an archived entry in a transaction's restore list is restored automatically. A premium-funded keeper is milestone 2 (9).

**2.5 Reentrancy is blocked by the host.** External calls default to `ContractReentryMode::Prohibited`, including indirect reentry. The design has no callbacks: the vault calls the pool and the strategies, they call only the token, and nothing calls back into the vault.

**2.6 Upgrades do not re-run constructors.** `update_current_contract` preserves address and storage (CAP-0058). The MVP has an admin-only `upgrade` and no state migration. A version-gated `migrate()` arrives with the multisig in milestone 3.

**2.7 Liquidity is thin and concentrated.** Aquarius carries about 98.7% of tracked Stellar DEX volume. Claims settle in the asset the pool already holds. Nothing in the system sells collateral on-ledger to fund a payout.

---

## 3. System overview

Two product contracts, CoverPool and CoveredVault, and a strategy interface. MockStrategy implements that interface for tests and the demo.

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

Nothing in the payout path leaves the ledger or reads a price.

**Flow.**

1. Stakers stake XLM into the CoverPool and receive shares under a lock.
2. The vault's curator buys cover. The pool reserves the limit and charges a premium, which goes to the stakers.
3. Depositors put XLM into the vault and receive shares. The vault records each depositor's principal.
4. The curator allocates idle XLM to whitelisted strategies.
5. If a strategy loses money, the vault's net asset value falls.
6. On withdrawal the vault values the depositor's shares, claims the difference between principal and value from the pool in the same transaction, and pays the depositor in full.
7. The claim lowers the pool's capital, so every staker is slashed pro rata through the share value.

**One contract, not a wrapper.** On Ethereum, Catalysis wrapped a Morpho vault. Stellar has no curated XLM vault to wrap, so the vault and the cover logic are one contract.

**Positions are ledger entries, not contract instances.** Soroban's instance storage shares the contract's TTL, so an instance per position would mean a thousand instances to keep alive for a thousand positions. Each position is a persistent entry keyed by address.

---

## 4. CoverPool

A staked capital pool that insures covered vaults. Source: [`contracts/cover_pool`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/cover_pool).

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

The whitelist of covered vaults is a set of instance-storage keys.

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

**Authorization.** Staker functions require the staker's auth. `add_vault` and `upgrade` require the admin. The vault-facing functions require the vault's own auth and the vault to be whitelisted. On Soroban a contract's authorization is implicit when it is the direct invoker, so `vault.require_auth()` passes only when the vault contract itself makes the call. A plain account cannot claim for a vault.

**No recipient parameter.** `claim` pays the vault that calls it, against that vault's own cover, capped at what remains. Nothing in the interface can direct a payout elsewhere.

### 4.3 Design decisions

- **Custody-lock, not allowance.** `stake` moves the asset into the pool's own balance. SEP-41 allowances are spending limits that do not lock funds, so a staker could revoke one when a loss looked likely.
- **Segregated pools.** A pool insures only the vaults whitelisted on it, so a staker chooses the risk they hold.
- **Fully collateralised.** Cover written never exceeds capital, and `unstake` refuses any withdrawal that would break that. A claim of `a` reduces capital and cover written by `a` each, so the inequality survives any number of claims in any order. The Ethereum system checked stake against the limit once, at binding.
- **Cover is reserved atomically.** `open_cover` reserves the limit and charges the premium in one call, and fails if capacity is short.
- **The pool prices its cover.** Premium is `limit × premium_bps / 10,000`, paid upfront by the curator and credited to stakers through the premium index. It is not part of `total_capital` and cannot be slashed.
- **Partial claims.** Each withdrawal with a shortfall is its own claim against the same cover. `close_cover` releases what is left and keeps the claimed total.
- **Dust is rejected.** A stake into a pool slashed to zero fails, and an unstake that would pay nothing fails. All other rounding is down, in the pool's favour.

**Invariants, enforced on every transition:**

```
coverage_written == Σ cover.remaining
coverage_written <= total_capital
claim amount     <= cover.remaining
```

---

## 5. CoveredVault

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

Shares are internal and non-transferable, which keeps per-address principal correct. A depositor who arrives after a loss buys shares at the lower net asset value, so their principal equals their value and they carry no shortfall.

### 5.2 The payout rule

```
value     = shares × nav / total_shares                 (rounds down)
protected = principal × shares / position.shares        (rounds down)
shortfall = max(protected − value, 0)
claimable = shortfall − shortfall × DEDUCTIBLE_BPS / 10,000
paid      = value + claimable
```

`withdraw` runs in this order: value the shares; pull `value` of liquidity, idle first and then strategies in whitelist order; claim `claimable` from the pool; update the books; pay the depositor. `nav` is idle plus what each strategy reports. It is never read from the vault's token balance, so the pool's payout passing through the vault cannot inflate it.

**Why at withdrawal.** A strategy loss lowers the vault's net asset value, and the vault's books say what each depositor is short. No price is read. The loss most worth covering is the one where an oracle has already failed, so the payout depends only on accounting. The Ethereum wrapper filed its claim the same way.

`DEDUCTIBLE_BPS` is a constant set to zero. It keeps the withdrawal maths in the same shape as the Ethereum wrapper's deductible.

### 5.3 Worked example

This is the integration test and the testnet demo. Every number is asserted.

| # | Step | Effect |
|---|---|---|
| 1 | Alice stakes 1,000 XLM into the CoverPool | Pool capital 1,000 |
| 2 | The curator buys 50 XLM of cover at 2.5% (1.25 XLM) | 50 reserved for the vault; premium credited to Alice |
| 3 | A user deposits 100 XLM; the curator allocates 50 to Market A and 50 to Market B | Vault NAV 100 |
| 4 | Market B loses 20% of its funds (10 XLM) | Vault NAV 90 |
| 5 | The user's position reads value 90, protected 100, shortfall 10 | View call |
| 6 | The user withdraws. The vault pulls 90 from the strategies and claims 10 from the pool | User receives 100; Alice's stake 1,000 → 990; remaining cover 50 → 40 |

A second integration test covers the path with no loss: the user withdraws what they deposited, with no claim and no slashing.

### 5.4 Strategies

A strategy exposes `total_value()` and `withdraw(amount)` and accepts XLM by transfer. The interface is in [`contracts/common`](https://github.com/0xcatalysis/catalysis-soroban/tree/main/contracts/common).

**MockStrategy, test and demo only.** Holds real XLM, so a simulated loss moves tokens and explorer balances match. `simulate_loss(bps)` is admin-only and sends that fraction of the balance to the admin. It must never hold real funds.

**Blend V2 strategy, milestone 1.** Supplies XLM through Blend's `submit` and values its position as `b_tokens × b_rate`, read through Blend's public `get_reserve` and `get_positions`. Blend socializes bad debt by lowering `b_rate`, so a socialized loss appears in `total_value()` and in every depositor's shortfall. The `Reserve` types for that read are already in `contracts/common`, mirrored from `blend-contracts-v2`.

---

## 6. Why the pool holds native XLM

**Finding.** We queried issuer authorization flags via Horizon for every Stellar asset attributable to a genuine issuer by `home_domain` and `stellar.toml`. Every verifiable issued asset is freezable by its issuer today or one signature away from it. No verified issuer has set `AUTH_IMMUTABLE`. USDC and EURC (Circle) are freezable. USDY (Ondo), BENJI (Franklin Templeton) and YLDS (Figure) can be frozen and clawed back. The dated snapshot, the queries and a re-run script are in `data/issuer-flags/` in the repository.

**Choice.** Native XLM has no issuer, no authorization flags, no freeze and no clawback. It needs no trustline and is the deepest asset on the network.

**Trade-off.** XLM's price tends to fall when the Stellar ecosystem is under stress, so the pool's capacity in dollar terms shrinks in those conditions. This is weaker than the backstop's exposure: BLND is the token of the one protocol it backstops and sits inside an AMM, while XLM is the base asset and is not the token of any covered protocol. Cover is written in the pool's own asset, so an XLM pool covers an XLM vault.

The asset is a constructor parameter on both contracts. A USDC vault and pool is milestone 2 and carries Circle's freeze right, disclosed per vault.

---

## 7. Loss accounting

A claim must cost the same number of writes whatever the number of stakers (2.3).

The pool holds `total_capital` against `total_shares`. A claim reduces capital and leaves every share count untouched, so the loss lands on all stakers pro rata with no per-position work.

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

Four writes: the cover entry, the pool state, and the two balances the transfer touches. Blend socializes losses the same way, by moving `b_rate` instead of touching balances.

Two alternatives were rejected. Epoch lists make a dormant staker walk every loss since they joined. A loss-per-share accumulator with share burning makes each holder's realized loss depend on when the others interacted.

Staking mints `amount × total_shares / total_capital`; unstaking pays `shares × total_capital / total_shares`. Both round down. The share price comes from internal accounting, so a direct transfer into the pool cannot inflate it.

**Premium** is claimed separately, so it uses an accumulator. On accrual `premium_index` advances by `amount × 10¹⁸ / total_shares`. A staker's earned premium is `shares × (premium_index − premium_index_entry) / 10¹⁸`, settled into their own entry whenever their share count changes.

---

## 8. Security

### 8.1 Testing

Unit tests run every contract function against the real neighbouring contracts. The worked example in 5.3 runs end to end with every number and every balance asserted. Scenarios were derived from the test suites of the audited Ethereum implementation and recomputed for this design: depositors at different entry prices, sequential losses, interleaved partial withdrawals, cover exhaustion, stakers joining and leaving between claims, premium conservation, and rounding at the stroop level. Scripted sequences check the invariants in 4.3 after every step. `scripts/check.sh` runs formatting, lints, tests and the Wasm build, and CI runs the same steps.

### 8.2 Admin authority

| Admin | Can | Cannot |
|---|---|---|
| Pool admin | Whitelist vaults; upgrade the pool | Alter a vault's cover; redirect or block a claim; withdraw staked capital; change share value except through a claim |
| Vault admin (curator) | Whitelist strategies; allocate idle deposits to them; buy and close cover; upgrade the vault | Touch a depositor's principal or shares; block a withdrawal; move deposits anywhere except a whitelisted strategy |

Depositors trust the curator's choice of strategies, as depositors in a Morpho vault do. Milestone 2 adds per-strategy allocation limits. The premium rate and the lock are fixed at deployment. Upgrade authority moves behind a multisig with a timelock in milestone 3.

### 8.3 Audit and licence

The Ethereum contracts were audited by Sigma Prime (Core, March 2026; Coverage, April 2026; combined, May 2026) and Certora (covered vault, March 2026). All four reports are public at https://docs.catalysis.network/docs/audits/.

The Soroban contracts are audited before mainnet using SCF audit credits, against the release tagged in milestone 2. They are published under Apache-2.0.

### 8.4 Known limitations of the MVP

| Limitation | Addressed in |
|---|---|
| Remaining cover is first-come. A withdrawal whose shortfall exceeds it fails. The MVP sizes cover above the maximum loss. | Milestone 1: pro-rata shortfall accounting |
| Stakers are slashed when a depositor withdraws, not when the loss occurs, so excess capital can leave in between. Capital backing cover cannot. | Milestone 1: loss reporting, notice period |
| Premium is paid upfront by the curator. Cover has no expiry. | Milestone 1 |
| Withdrawals drain strategies in whitelist order. No allocation limits. | Milestone 2 |
| Zero-share mints are rejected, but there is no virtual share offset against donation-based share inflation. | Before audit |
| A vault with zero NAV or a pool slashed to zero rejects new deposits. | Redeploy |
| Each admin can upgrade its contract. | Milestone 3: multisig, timelock |

---

## 9. Milestones

Three milestones, the last one mainnet. Each completion test is something the team can make true alone.

### Milestone 1 — 15 February 2027

| Deliverable | Done when |
|---|---|
| **Blend V2 strategy.** Replaces the mock with a strategy that supplies to a real Blend V2 pool and values its position through `b_rate`. | The demo runs on testnet against a real Blend V2 pool, with the loss coming from an actual `b_rate` decline. |
| **Loss reporting and staker notice period.** The vault reports losses to the pool as they happen, remaining cover is shared pro rata across depositors, and unstaking requires notice so it cannot front-run a loss. | Tests and a testnet run show stakers slashed before any depositor withdraws, and two depositors short of cover each paid the same fraction. |
| **Streaming premium, cover expiry and property tests.** Premium accrues per ledger from vault yield, cover ends at its expiry ledger, and pool and vault invariants are tested across randomized sequences. | The tests pass and run in CI on every push. |

### Milestone 2 — 15 March 2027

| Deliverable | Done when |
|---|---|
| **Multiple Blend markets with enforced allocation limits.** A vault allocates across several Blend markets under per-strategy limits the contract enforces. | A testnet vault holds positions in two markets and an over-limit allocation is rejected. |
| **TTL keeper and USDC vault.** An open-source keeper, funded from premium, keeps every position's ledger entry alive. A USDC vault and pool run alongside XLM, with Circle's freeze right disclosed. | The keeper carries every position through a full TTL cycle on testnet, and the demo runs end to end on USDC. |
| **STRIDE threat model and on-chain monitoring plan**, on SDF's templates. The threat model has a dataflow diagram with trust boundaries, at least one threat per STRIDE category, and a remediation for each. The monitoring plan lists every deployed component, the on-chain effect of each threat, and a monitor rule and alert owner per effect. | Both documents are published in the repository, with every monitor rule traced to a threat ID and every component listed at its testnet address. |
| **Feature freeze and audit scope.** Contracts frozen and tagged, with the threat model handed to the auditor. | A tagged release exists and the audit is scheduled against it. |

### Milestone 3 — 15 April 2027

| Deliverable | Done when |
|---|---|
| **Audit remediation, multisig and timelock.** Every finding closed and the report published. Every admin function behind a multisig with a timelock. | No unresolved high or critical findings, and admin control is verified on the ledger. |
| **Mainnet launch with a live covered vault.** Pool, vault and Blend strategy deployed to mainnet and operated by the team: stake posted, cover open, vault allocated to a Blend market. | The team has staked, bought cover, deposited, allocated and withdrawn on mainnet, with every transaction linked. |
| **SDK and integration documentation**, for curators offering a covered vault and stakers underwriting one. | The SDK is published with documentation and the demo script uses it. |

---

## 10. Team

**Abhishek Kumar, CEO.** Ex-Product at Obol (DV Labs), protocol engineering at Nethermind, previously Amazon and Atlassian. Ethereum Proof-of-Stake, staking infrastructure, DeFi.

**Dhruv Bodani, CTO.** Ex-Tech Lead for L2s at Obol (DV Labs), Senior Engineer at Offchain Labs (Arbitrum), protocol engineering at Nethermind, previously Visa and Atlassian. Distributed and P2P systems, Ethereum PoS, DeFi.

**Track record.** We designed, audited and shipped Catalysis to Ethereum mainnet: vault-native risk coverage over Gauntlet's WETH Prime vault on Morpho, with underwriting capacity from EigenLayer and Symbiotic. Both founders co-authored *Economic Security of Multiple Shared Security Protocols* (Marble 2025, Imperial College Brevan Howard Centre for Financial Analysis).

**Continuity.** Catalysis concludes its Ethereum operations in October 2026. The work continues on Stellar under Catalyst Labs LLC FZ, the contracting entity for this award.
