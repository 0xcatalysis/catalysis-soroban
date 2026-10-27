# Catalysis on Soroban

**Covered vaults for Stellar.** An XLM vault whose depositors are insured by a staked XLM pool.

Depositors put XLM into a CoveredVault. The curator allocates it to strategies (Blend markets, mocked for now) and buys cover from a CoverPool funded by stakers. If a strategy loses money, a depositor still withdraws in full: the vault claims the shortfall from the pool in the same transaction, and every staker is slashed pro rata. No trigger, no oracle, no claims committee. The payout is the vault's own accounting.

This is the Soroban implementation of [Catalysis](https://catalysis.network), which shipped to Ethereum mainnet and was audited by Sigma Prime and Certora. Built for Stellar Community Fund #46.

| Document | What it is |
|---|---|
| [Technical architecture](https://0xcatalysis.github.io/catalysis-soroban/) (also [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)) | The reviewer document: the problem, what exists, the Stellar constraints, contracts, accounting, security, milestones, growth metrics, team |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Every design decision made during the build, with the reason and what was rejected |
| [`docs/BACKGROUND.md`](docs/BACKGROUND.md) | The research behind the architecture: the February 2026 incident forensics, the full Stellar constraints list, issuer-flag evidence, rent and TTL, the Blend, DeFindex and oracle read surfaces, the threat table, and the full known-limitations list |
| [`docs/PLAN.md`](docs/PLAN.md) | The build plan, the founders' fixed decisions, the task log and the known-limitations list |
| [`CLAUDE.md`](CLAUDE.md) | Rules and conventions for anyone (or any agent) working in this repo |

> **Status: pre-grant MVP.** The happy path runs end to end in 91 tests and on Stellar testnet. The extensions scheduled for the grant milestones are listed in [Scope](#scope). The code is not audited. Do not use it with real funds.

## How it works

```
                    deposit / withdraw (paid in full)
   depositor ─────────────────────────────────────► CoveredVault
                                                     │   │   ▲
                                 allocate XLM        │   │   │ claim shortfall
                        ┌────────────────────────────┘   │   │ (at withdrawal)
                        ▼                                │   │
                 Strategy A      Strategy B              │   │
               ("Blend market")("Blend market")          │   │
                                                         │   │
                                    buy_cover / premium  ▼   │
   stakers ──── stake XLM ─────────────────────────► CoverPool
                                                   (slashed pro rata on claim)
```

The demo scenario, which is also the integration test and the testnet script:

1. Alice stakes 1,000 XLM into the CoverPool.
2. The curator buys 50 XLM of cover for the vault, paying a 2.5% premium (1.25 XLM) that goes to Alice.
3. A user deposits 100 XLM. The curator allocates 50 to Market A and 50 to Market B.
4. Market B fails and loses 20% of its funds (10 XLM). Vault NAV is 90.
5. The user's position reads value 90, protected 100, shortfall 10.
6. The user withdraws and receives the full 100 XLM. The pool pays the 10 XLM shortfall. Alice's stake goes from 1,000 to 990 and the vault's cover from 50 to 40.

| Contract | Role |
|---|---|
| [`cover_pool`](contracts/cover_pool/src/lib.rs) | Staked capital. Mints shares, reserves cover for whitelisted vaults, prices and distributes premium through a per-share accumulator, and pays claims by lowering the share value, so every staker is slashed pro rata in a constant number of writes. A vault can claim many times against one cover. Fully collateralised: cover written never exceeds capital, and capital backing live cover cannot be unstaked. |
| [`covered_vault`](contracts/covered_vault/src/lib.rs) | The vault. Internal non-transferable shares and per-depositor principal (what the shares were worth when bought, so rounding is never claimed). The curator whitelists strategies, allocates idle XLM to them and buys cover. `withdraw` values the shares, pulls liquidity, claims any shortfall against principal from the pool, and pays the depositor in full. A deductible constant exists and is zero. |
| [`mock_strategy`](contracts/mock_strategy/src/lib.rs) | **Test and demo only.** Stands in for a Blend market. Holds real XLM so a simulated loss really moves tokens out. `simulate_loss(bps)` is admin-only. Milestone 1 replaces it with a strategy that supplies to a real Blend V2 pool. |
| [`common`](contracts/common/src/lib.rs) | Cross-contract interfaces (the pool and strategies as the vault sees them) and the Blend V2 types, mirrored field for field from `blend-capital/blend-contracts-v2`, for the milestone-1 Blend strategy. |

**Why XLM.** Native XLM has no issuer, so nobody can freeze or claw back the pool's capital. That matters on Stellar, where an issuer's freeze and clawback rights reach balances held by contracts (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §6 and [`data/issuer-flags/`](data/issuer-flags/)). The asset is a constructor parameter on both contracts, so a USDC vault and pool is a redeploy rather than a code change.

**Why payout at withdrawal.** It is how Catalysis works on Ethereum, and it needs no trigger and no oracle. A loss in a strategy lowers the vault's NAV; the vault's own books say exactly what each depositor is short. The loss we most want to cover is the one where an oracle has already failed, so the payout depends on nothing but the accounting.

## How the contracts talk

```
depositor ──withdraw──► CoveredVault ──withdraw(amount)──► Strategy ──transfer──► vault
                            │
                            ├──claim(vault, shortfall)──► CoverPool ──transfer──► vault
                            │
                            └──transfer(value + shortfall)──► depositor
```

- **Calls only go outward.** The vault calls the pool and the strategies; they call only the XLM token contract. Nothing calls back into the vault, so Soroban's no-reentrancy rule is never hit.
- **Contract authorization is implicit on direct calls.** When the vault invokes the pool, the pool's `vault.require_auth()` passes because the vault is the direct invoker. A plain account cannot claim on a vault's behalf. This is why `buy_cover` and `close_cover` exist on the vault: the pool insists on the vault's own signature.
- **The curator signs once for cover.** Their signature on `buy_cover` covers the nested premium transfer from their account to the pool.
- **Interfaces live in [`contracts/common`](contracts/common/src/lib.rs)** as `#[contractclient]` traits, so no contract depends on another's crate and no exports leak between WASMs.

**Accounting rules.** Amounts are in stroops (1 XLM = 10,000,000). Share and value maths round down, in the contract's favour. Operations that would mint zero shares or pay zero fail rather than round. NAV is idle XLM plus what every strategy reports; it is never read from the vault's own token balance, so the pool's payout passing through the vault cannot inflate it.

**Storage.** Config and aggregate state are in instance storage. Each staker position, depositor position and vault cover is a persistent entry whose TTL is extended on every write, and anyone can extend a position with `bump_position`.

## Repository layout

```
contracts/
  cover_pool/       staked XLM insurance pool
  covered_vault/    the vault
  mock_strategy/    TEST AND DEMO ONLY stand-in for a Blend market
  common/           cross-contract interfaces + Blend V2 types for milestone 1
integration-tests/  the demo scenario and the quiet path, end to end
scripts/
  check.sh          local CI: fmt, clippy, tests, WASM build
  demo_testnet.sh   the demo on Stellar testnet
deployments/        testnet contract IDs, written by the demo script
data/issuer-flags/  ledger evidence for "why native XLM", with a re-run script
docs/               ARCHITECTURE.md, DECISIONS.md, PLAN.md
.github/workflows/  ci.yml (manual until submission), pages.yml (publishes the architecture doc)
```

## Run the tests

```sh
./scripts/check.sh      # fmt, clippy, all tests, WASM build: the same steps as CI
cargo test --workspace  # tests only
```

Needs Rust stable with the `wasm32v1-none` target and the [Stellar CLI](https://developers.stellar.org/docs/tools/cli) 28 or later. soroban-sdk 28 requires contracts to be built through `stellar contract build`; a plain `cargo build --target wasm32v1-none` is refused by the SDK's build script.

| Crate | Tests | What they cover |
|---|---|---|
| `cover_pool` | 34 | Staking, the lock, capacity, premium split and conservation, partial and sequential claims, two vaults sharing a pool, stakers leaving between claims, rounding, invariants |
| `covered_vault` | 47 | Share pricing, NAV, the demo scenario, partial and total losses, multiple depositors at different entry prices, sequential losses, interleaved withdrawals, cover exhaustion, rounding, invariants, a 400-operation sequence with no loss and no claim |
| `mock_strategy` | 8 | Value, withdrawal, simulated loss, bounds, authorization |
| `integration-tests` | 2 | [`covered_vault_demo.rs`](integration-tests/tests/covered_vault_demo.rs): the six steps above with every number asserted and every balance reconciled, plus the quiet path with no loss |

The scenario tests were derived from the Ethereum Catalysis test suites and recomputed for this MVP's simpler rules.

## Run the demo on testnet

Needs the Stellar CLI and `jq`.

```sh
./scripts/demo_testnet.sh                # full run, about four minutes
DEMO_PAUSE=1 ./scripts/demo_testnet.sh   # pause between steps, for recording
```

The script builds the contracts, funds three accounts with Friendbot (curator, Alice, user), deploys the pool, the vault and two mock strategies, runs the six steps above, and finishes with Alice collecting her premium and unstaking 990 XLM after the lock. Contract IDs are written to [`deployments/testnet.json`](deployments/testnet.json). Stellar resets testnet periodically; re-run the script to redeploy.

## Testnet deployments

Deployed 25 September 2026.

| Contract | ID |
|---|---|
| CoverPool | [`CB3SK2KHIBOKV3WHBMXMJOYQJDSGCXJ6GGFZZN5UZM63K25B5GTBNVIO`](https://stellar.expert/explorer/testnet/contract/CB3SK2KHIBOKV3WHBMXMJOYQJDSGCXJ6GGFZZN5UZM63K25B5GTBNVIO) |
| CoveredVault | [`CDMDOL3XKEHHUOCX7M7QRLCBH6AULKHGHGTKIQXOW6HSC7GJKLBAU27A`](https://stellar.expert/explorer/testnet/contract/CDMDOL3XKEHHUOCX7M7QRLCBH6AULKHGHGTKIQXOW6HSC7GJKLBAU27A) |
| MockStrategy A | [`CAVB427ZBHTTDCC7TMKWQZIBDAUQTMEXASF2KAQ7HMB26TM3TCJEH6II`](https://stellar.expert/explorer/testnet/contract/CAVB427ZBHTTDCC7TMKWQZIBDAUQTMEXASF2KAQ7HMB26TM3TCJEH6II) |
| MockStrategy B | [`CAK4WT6UOXAXLBZRN37X6WHXBEUREQ4G6YDQHJWIGQMUVDUZVBX4WFF6`](https://stellar.expert/explorer/testnet/contract/CAK4WT6UOXAXLBZRN37X6WHXBEUREQ4G6YDQHJWIGQMUVDUZVBX4WFF6) |

## Development

- `soroban-sdk 28.0.0` is the only dependency. Contracts are `#![no_std]` with `crate-type = ["cdylib", "rlib"]`.
- Run `./scripts/check.sh` before every commit. It is the same sequence as the CI workflow, which runs on every push to `main` and every pull request.
- Conventions (errors, events, upgrades, storage, auth in tests) are in [`CLAUDE.md`](CLAUDE.md). Unit tests register the real pool and strategy contracts through dev-dependencies, which never reach the WASM.
- Mocks say **TEST AND DEMO ONLY** in their doc comments, here, and in the architecture doc.

## Scope

Built now (pre-grant MVP): everything above. Known limitations are listed in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §8.4, each with the milestone that addresses it.

Scheduled for the grant milestones, per [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §9:

- **Milestone 1, 15 Feb 2027:** a real Blend V2 strategy on testnet valued through `b_rate`; loss reporting with pro-rata shortfall accounting and a staker notice period; cover expiry and streaming premium from vault yield; property tests
- **Milestone 2, 15 Mar 2027:** multiple Blend markets with curator allocation limits; keeper-driven TTL maintenance; USDC vault and pool; STRIDE threat model and monitoring plan; audit engaged
- **Milestone 3, 15 Apr 2027:** audit remediation; multisig and timelock; mainnet; a covered vault live on mainnet, operated by the team; SDK and docs

A DeFindex strategy and price-based cover for positions with no accounting signal follow mainnet and are outside this grant.

## License

[Apache-2.0](LICENSE).

*Contract code, tests and documentation in this repository were written with AI assistance and reviewed by the Catalysis team, who own every design decision in it.*
