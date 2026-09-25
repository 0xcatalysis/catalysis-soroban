# Design decisions

A log of the decisions made while building the covered-vault MVP, so nobody has to reconstruct them from git history or chat. Founders' decisions that predate the build are in [`PLAN.md`](PLAN.md) §2 and are not repeated here. Everything below was decided on 25 September 2026 during the build session unless marked otherwise, and each entry says what was decided, why, and what was rejected.

Read this alongside [`ARCHITECTURE.md`](ARCHITECTURE.md) (the design as shipped) and [`PLAN.md`](PLAN.md) §7 (known limitations).

---

## 1. Scope and product

**1.1 The vault is a Morpho-style vault with insurance built in, not a wrapper.** Ethereum Catalysis wraps an existing Morpho vault. Stellar has no curated XLM vault product to wrap, so the CoveredVault holds deposits, lets the curator allocate to strategies, and pays shortfalls itself. Rejected: wrapping DeFindex, which would add a second share layer and an external dependency to a four-week MVP. Founders confirmed.

**1.2 The deductible exists as a constant and is zero.** `DEDUCTIBLE_BPS = 0` in the vault, applied in the withdrawal maths exactly where the Ethereum wrapper applies its deductible, so the shape matches without changing the demo numbers. Not configurable. Founders' decision.

**1.3 Cover exhaustion fails the withdrawal outright.** When a shortfall exceeds the vault's remaining cover, the pool's claim errors and the whole withdrawal reverts with state unchanged. Ethereum silently caps the payout instead. The revert is simpler and never pays partial cover by accident; the depositor can withdraw fewer shares or wait for a top-up. Documented in `PLAN.md` §7 and `ARCHITECTURE.md` §10.5.

**1.4 Four small guards were added rather than left as known limitations.** Deposits that would mint zero shares fail (`ZeroShares`); deposits into a vault with zero NAV fail (`VaultDrained`); stakes into a fully slashed pool fail (`PoolDrained`); withdrawals or unstakes that would pay nothing fail (`InvalidAmount`). Each is two or three lines and turns a silent divide-by-zero or a free share burn into a named error. Surfaced by scenarios ported from the Ethereum test suites.

**1.5 Share inflation by donation is only partly mitigated.** Rejecting zero-share mints stops the first step of the attack; a full defence needs a virtual share offset or minimum first deposit, as in the Ethereum wrapper's decimals offset. Left for milestone 1 because principal protection makes the attack a way to drain the pool, which deserves a designed fix rather than a rushed one.

**1.6 Ethereum features deliberately omitted.** Streaming premium and its platform/pool fee split, the curator review gate between claims, the per-depositor loss cap, deposit caps, emergency exit, cover expiry. Each is listed in `PLAN.md` §7 with the Ethereum mechanism it corresponds to. Cover top-up is the one place the MVP is looser than Ethereum: `open_cover` adds to remaining cover rather than requiring a new policy.

## 2. Repository, tooling and process

**2.1 Hosted under `github.com/0xcatalysis`, not a personal account.** The SCF form's project is Catalysis, its team description sells the Catalysis mainnet track record, and the `core` and `coverage` repos live in the org. A personal repo would break that story. Private until submission, matching the org's other unshipped repos; making it public is a pre-submit TODO in `PLAN.md` Task 8. Founders' decision.

**2.2 CI runs locally; the GitHub workflow is manual-only.** `scripts/check.sh` runs fmt, clippy, tests and the WASM build in the same order as `.github/workflows/ci.yml`. The workflow is `workflow_dispatch` to save Actions minutes on the private repo; restoring push triggers is a pre-submit TODO. Founders' decision. One dispatched run proved the workflow green before tagging.

**2.3 CI builds WASMs with the Stellar CLI, pinned to 28.0.0.** soroban-sdk 28 refuses a plain `cargo build --target wasm32v1-none` for contracts and requires `stellar contract build`. The official `stellar/stellar-cli` action installs it. Found when the first two CI runs failed.

**2.4 Task order: delete the old contracts first.** The original plan changed the pool API before deleting the Blend-policy contracts that called it, which would have left the workspace broken across two tasks. Reordered so every task ends with the whole workspace green.

**2.5 Cross-contract calls go through client traits in `contracts/common`; tests use the real contracts as dev-dependencies.** The traits keep contract exports from leaking between WASMs (verified: the vault WASM exports only its own 14 functions). Dev-dependencies never reach the WASM, so unit tests can register the real CoverPool and MockStrategy and exercise the actual auth and token flows instead of hand-written mocks.

**2.6 Error codes are kept stable where tests already reference them.** Pool codes 5, 7 and 9 kept their meaning across the API change; new codes took the freed numbers. Cosmetic, but it kept the staking tests untouched through the rewrite.

**2.7 `BPS` is a private constant in the pool** rather than imported from `common`. The pool has no dependency on `common` and one constant does not justify adding one.

**2.8 No GitHub release was cut.** A release is public-facing metadata on a private repo. The annotated tag `v0.1.0-mvp` is enough for the submission to reference; a release can follow the visibility flip.

**2.9 The architecture doc is published with GitHub Pages from `main`.** The SCF form needs an accessible link. On the org's Team plan, Pages from a private repo is public. A workflow prepends front matter to `docs/ARCHITECTURE.md` and builds it with Jekyll's Primer theme, so the source markdown stays clean and GitHub still renders it normally.

## 3. CoverPool

**3.1 Cover is keyed by vault address and supports many partial claims.** Each withdrawal with a shortfall is its own claim, as on Ethereum where `claimId = claimCount++`. `remaining` falls and `claimed` rises.

**3.2 `close_cover` keeps the cover entry with `claimed` intact** and zeroes `remaining`, rather than deleting it. The demo and explorer can still show what a vault was paid after its cover is closed. A vault that never opened cover reads as zeros instead of erroring. Matches Ethereum, where nothing is deleted.

**3.3 The whitelist is checked on all three vault-facing calls**, not only `open_cover`. Stricter than Ethereum, which re-checks identity at claim time but not the pool registry. Behaviourally identical while there is no `remove_vault`.

**3.4 The whitelist lives in instance storage**, one key per vault; the per-vault `Cover` entry is persistent with its TTL extended on write. Small admin-managed config belongs in instance storage per the repo conventions.

**3.5 The claimant is the vault itself and there is no recipient parameter.** `claim` pays the calling vault against its own cover. Soroban's implicit authorization for a direct contract invoker means only the vault contract can pass `vault.require_auth()`. This replaced the old "beneficiary fixed at reservation" argument, which rested on reentrancy.

**3.6 Zero premium skips the token transfer** but still reserves cover. Lets a deployment set `premium_bps` to zero without a zero-amount transfer. With any positive limit the capacity check guarantees shares exist, so the accumulator update is safe.

**3.7 `NoCover` is distinct from `ClaimExceedsCover`**, so a vault can tell "never insured" from "insured but exhausted". Ethereum distinguishes `PolicyNotFound` from the over-limit error the same way.

**3.8 Events carry the numbers the demo narrates.** `Slashed` has the vault, amount, remaining cover and capital after; `CoverOpened` has limit, premium and remaining after. Ethereum core's `SlashingExecuted` carries no amounts, which its own docs note as a gap.

**3.9 `available_capacity` and `is_vault` views stay** even though no contract calls them. The demo script prints capacity and can confirm whitelisting before buying cover.

**3.10 Premium is credited through the per-share index and never enters `total_capital`**, so it cannot be slashed. Matches Ethereum, where rewards go to reward contracts, never into vault deposits.

## 4. MockStrategy

**4.1 The simulated loss goes to the admin, not burned.** On testnet the admin account visibly gains what the strategy lost, which reads on the explorer as "the market took the money". The XLM token contract has no burn for contract-held balances without extra auth plumbing.

**4.2 `simulate_loss` returns the amount moved** and emits `LossSimulated` with bps, amount and value after. The plan had it returning nothing. Lets the script and tests assert the exact 10 XLM without a separate balance read.

**4.3 Loss of 100% is allowed; 0% and over 100% are rejected.** A total loss is the zero-NAV case the vault refuses to accept deposits into, and the strategy must be able to produce it so that limitation can be demonstrated.

**4.4 `withdraw` checks the balance itself** and fails with `InsufficientBalance` rather than letting the token contract fail with its own error, so the vault gets a clear code if accounting ever drifts.

**4.5 No dependency on `common`.** The strategy matches `StrategyInterface` by signature. Implementing the trait would pull `common` into the WASM for no benefit.

**4.6 No TTL bumping.** Only instance storage, which the vault's calls extend in practice.

## 5. CoveredVault

**5.1 Shares are priced from the strategies' reported values, not the vault's token balance.** `idle` is bookkeeping state. The pool's claim passes through the vault's balance on its way to the depositor without inflating NAV. This is why the late-depositor and partial-withdrawal numbers come out exact.

**5.2 Liquidity is pulled from idle first, then strategies in whitelist order**, taking each to zero before moving on. Simplest rule that always succeeds, since value never exceeds NAV. A small withdrawal can drain Market A while Market B is untouched; a pro-rata pull is a milestone-1 refinement.

**5.3 Call order in `withdraw`: value the shares, pull liquidity, claim from the pool, update the books, pay.** All cross-contract calls go outward from the vault. Nothing calls back in, so Soroban's no-reentrancy rule is never hit. Books are updated before the final transfer.

**5.4 `buy_cover` takes only a limit.** The premium rate belongs to the pool, so the vault passes the curator as payer and reports back what was charged.

**5.5 `close_cover` on the vault, admin-gated, calling the pool.** The pool requires the vault's own signature and no CLI account can provide it. Found on the first testnet run; the unit tests had hidden it because mocked auth let the curator stand in for the vault. Lesson: anything only the vault should call needs a vault entry point, and mocked auth will not tell you.

**5.6 `allocate` rejects unknown strategies and amounts above idle** with distinct error codes, so the demo script gets a clear failure if it allocates in the wrong order.

**5.7 A depositor with no position reads as zeros** rather than erroring, so a position can be printed before the first deposit.

**5.8 `upgrade` and `bump_position` included** to match the pool's operational surface and the repo's conventions on upgrades and TTL.

**5.9 `Withdrawn` carries shares, value and shortfall paid separately**, so the explorer shows "90 from the vault, 10 from the pool".

## 6. Tests

**6.1 Scenarios were derived from the Ethereum Catalysis suites.** Two explorer passes over the CoveredVaultWrapper, ClaimManager, CoverPool, PremiumManager, RewardsManager and StakeManager tests produced scenario tables; each was recomputed for the MVP's simpler rules (no premium streaming, cap or deductible) and ported as a unit test. Everything needing an omitted feature was skipped and listed. Every ported scenario passed against the existing maths; the only contract changes were the guards in 1.4.

**6.2 Multi-order scenarios run both orders** (two equal depositors; a late depositor through two losses) in a loop, so a bug that depends on withdrawal order cannot hide.

**6.3 Scripted invariant tests instead of property tests.** A fixed sequence of a dozen steps with every invariant checked after each step. Randomized property tests are milestone 1; the scripted version costs nothing extra and already catches accounting drift.

**6.4 Every integration-test balance is reconciled**, not only the ones the demo table names: user, Alice, pool, vault, both markets and the curator. If any contract leaks a stroop, the test finds it.

**6.5 The integration test has an epilogue past the demo table**: Alice collects her premium, is refused a full unstake while cover is still reserved, and unstakes 990 once the curator closes the cover. It shows full collateralisation biting on real numbers, and it is what caught 5.5.

**6.6 Test helpers fund the curator with the full limit** rather than the exact premium, so staking tests do not care about premium arithmetic; one premium-specific test funds exactly and asserts the payer ends at zero.

## 7. Testnet demo

**7.1 Three identities: curator, alice, user.** The curator is admin of every contract. Created once by the script with Friendbot and reused on reruns; they live in the CLI's config directory, which is gitignored.

**7.2 Lock is 12 ledgers on testnet**, about a minute, so the epilogue fits in a recording. Unit tests use their own constants.

**7.3 The script mirrors the plan's §1 table one step per block**, with the same wording as the integration test's comments, so the two can be checked against each other line by line.

**7.4 Redeploy after the last contract change.** The IDs in the README are from the run made against the final WASMs, so the explorer shows the shipped code. Stellar resets testnet periodically; re-run before recording the video and before submission.

## 8. Documentation

**8.1 `ARCHITECTURE.md` keeps its sourced sections and replaces only what the design changed.** Sections 1, 2, 6, 8, 9.2, 12, 13 and 14 are the grant's research and stand. The Blend read-surface material in §9.1 was kept and reframed as how the milestone-1 strategy values its position; its code sample is marked milestone 1.

**8.2 Known limitations are listed in both `PLAN.md` §7 and `ARCHITECTURE.md` §10.5.** The architecture doc is what reviewers read; the plan is the internal handoff. If one changes, change both.

**8.3 Admin authority is stated honestly**, including the curator's power to allocate. Depositors trust the curator's strategy choice, exactly as on Morpho.

**8.4 The milestone table in `ARCHITECTURE.md` §11 is left as the old scope with a marked `TODO(founders)`** and the plan's §9 proposal summarised beneath it. Milestone scope is the founders' call.

**8.5 Every claim in the docs matches the code.** Where the code lacks something the doc used to claim (a `migrate()` function, a fixed beneficiary), the doc now says so and names the milestone.
