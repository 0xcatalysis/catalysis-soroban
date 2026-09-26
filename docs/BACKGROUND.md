# Catalysis on Soroban — Background and research notes

Supporting material for the [technical architecture document](ARCHITECTURE.md): the incident forensics, the full list of Stellar constraints, what we found on the ledger about issuer flags, rent and TTL, the Blend and DeFindex read surfaces, the oracle-trigger design that follows mainnet, the threat table, and the full known-limitations list. Everything here was verified against primary sources on 25 September 2026. Section references in parentheses of the form 2.x point to the constraints list in part B below; all others point to the architecture document.

---

## A. The February 2026 incident, and why the existing buffer cannot be the answer

### A.1 What happened on 22 February 2026

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

### A.2 Why the existing buffer cannot be the answer

Blend's backstop is the only bad-debt absorption mechanism of consequence on Stellar. It is denominated in **BLND:USDC 80:20 liquidity pool shares**, per Blend's documentation. The contract's backstop threshold test computes `(bal_blnd⁴ × bal_usdc) / PC⁵` against `1e25` with `PC = 100_000`; the 4:1 exponent ratio mirrors those Comet pool weights.

This is wrong-way risk, and it is not hypothetical:

- **The collateral is mostly the governance token of the protocol being backstopped.** BLND trades at $0.0053, down about 95.6% from its November 2024 high of $0.12 and down 37% over thirty days. Capacity denominated in that asset degrades exactly as confidence in the protocol degrades.
- **The collateral lives in an AMM pool whose bugs are the backstop's bugs.** In late August 2026 a vulnerability in same-asset swaps in the Comet AMM — the BLND-USDC pool collateralising the backstop — was reported to have caused a loss of roughly $717,000, and Blend's operators were reported to have paused the backstop. That figure and the pause come from a single originating outlet (DailyCoin, 28 August 2026, syndicated onward), unconfirmed by any security firm or by Script3, so we hold them loosely. The bug class is independently corroborated in source: Comet v1's error enum carries a dedicated `ErrSameToken` variant alongside overflow and minimum-balance guards. We could not locate a release note describing the fix and do not assert one.
- **It is not quickly exitable, and its exit is observable.** Backstop withdrawals run through a 17-day queue, and `q4w_pct` on the backstop's pool data exposes how much is already queued.

A first-loss buffer bounded in practice by the depth of its own collateral, held inside a contract that is itself a solvency surface, is not a risk-transfer system. It is a deductible.

*(Contemporaneous reporting also claimed Stellar TVL fell 60% in a single day to $98M and Blend's TVL fell "to nearly zero." DefiLlama's own historical series contradicts both — Stellar chain TVL was $244.6M on 27 August against a 22 August peak of $264.9M, and Blend was $152.8M. We do not repeat those figures.)*

---

## B. The constraints that shape this design, in full

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

---

## C. Issuer flags on the ledger, and issued assets later

### C.1 What we found on the ledger

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

### C.2 Issued assets later

Adding issued assets needs machinery the MVP deliberately leaves out.

- **An attested flag registry.** A contract cannot read issuer flags (2.8), so a keeper writes them on-ledger with a ledger stamp. A stale or regressed attestation halts new deposits and never blocks a payout.
- **A live `authorized()` check** on the pool's own balance in the settlement path.
- **A rule that the pool's balance entry must never close.** Clawback attaches when a balance entry is created, so an entry that exists before an issuer enables clawback stays outside it.

Yield-bearing RWA capacity comes after the grant. It would need par valuation from the issuer's attestation, never a Stellar venue price: in February a treasury token with under $1 of hourly volume was marked at a manipulated price. Draw-downs would settle by redemption against the issuer, never by selling into a thin venue (2.11).

---

---

## D. Rent, TTL and liveness

Staker positions must survive a lock period during which the staker does nothing, and depositor positions must survive for as long as the deposit sits in the vault. Both are persistent entries with independent TTLs (2.5), as is each vault's cover entry in the pool.

**Today.** Every write extends the entry it touches, so any position or cover that is interacted with stays live. `bump_position` on the pool and on the vault lets anyone extend a position entry without its owner's signature.

**Milestone 2: a funded keeper.** A standing deduction from premium funds TTL maintenance. The position exists because cover was sold, so the premium that cover generates keeps it alive. Stakers are not asked to fund rent on a position they cannot touch during the lock, and depositors in covered vaults are not charged for our storage at all.

TTL extension has no access control, so anyone may extend any entry. We will publish the keeper so third parties can run one.

**Why correctness does not depend on it.** The docs warn against relying on TTL extension for safety, and we do not. From protocol 23 an archived persistent entry included in a transaction's restore list is restored automatically when the invocation executes (2.5), so a claim against an archived position succeeds without a separate operation — it costs more in restore fees and footprint, but it does not fail. The keeper keeps claims cheap; a claim is correct whether or not it ran.

Rent pricing is itself a live network parameter rather than a constant — stellar-core's persistent rent-rate denominator changed materially at protocol 23 — so our fee model reads current values from the network rather than embedding them. Rent is an operating cost with no EVM analogue and appears as an explicit line in the budget submitted with the application.

---

---

## E. Blend, DeFindex and oracle triggers: what is decidable on-ledger

The MVP needs no trigger. The payout condition is the vault's own accounting (5.2): a depositor is paid whatever their shares are worth plus whatever that falls short of their principal. This section covers how the milestone-1 Blend strategy will value its position, which is where Blend's on-ledger state enters the design, and the oracle-derived triggers for positions that have no accounting signal, which follow mainnet (11).

### E.1 A Blend strategy's value is accounting, not a price

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

### E.2 Oracle-derived triggers, and where they are not available

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

---

## F. Threat model

The threat table from earlier versions of the architecture document, kept here as the starting point for the milestone-2 STRIDE threat model. Section references point to the architecture document.

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
| Vault share price inflated by donating to a strategy | Zero-share mints rejected; virtual share offset or minimum first deposit to follow (8.5) | Partial |
| Rounding drift | Mint, redeem and shortfall maths round down, in the contract's favour; dust operations rejected (4.3, 5.2) | Implemented |
| Oracle manipulation to suppress or force a payout | The payout reads no price; it is the vault's accounting (5.2) | Implemented |
| Cross-contract reentrancy | Blocked by the host (2.9); the vault calls out, nothing calls back in (4.2) | Implemented |
| Staker flight ahead of a known loss | Lock and full collateralisation today; loss reporting and a notice period (4.4) | Milestone 1 |
| Cover exhausted, later withdrawers unpaid | Cover sized above maximum loss today; pro-rata shortfall accounting (8.5) | Milestone 1 |
| Curator allocates to a bad strategy | Curator whitelists strategies; depositors trust the curator, as on Morpho; allocation limits (9) | Milestone 2 |
| Position archived when a withdrawal or claim lands | Protocol-23 automatic restore; interactions extend TTL today, keeper in milestone 2 (9) | Partial |
| Admin key compromise | 10.3 | Before mainnet |

---

## G. Known MVP limitations, in full

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

---

## H. On-chain growth, and how it is measured

Kept for the SCF form's question on how growth is driven and measured.

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

The monitoring plan in milestone 2 publishes a dashboard over exactly these fields. The mainnet vault in milestone 3 is the first entry in it. What counts as success is the four balances that a covered vault moves on-ledger: capital staked, cover written, deposits covered, and premium paid, all in XLM, all from contract state.

---
