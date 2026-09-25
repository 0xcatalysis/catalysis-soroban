# Catalysis on Soroban

**Underwritten bad-debt coverage for Stellar credit markets.**

Stakers put XLM into a CoverPool. Vaults and lenders buy cover on their Blend supply positions. When Blend socializes bad debt across a reserve's suppliers, the covered loss is paid out automatically. Anyone can trigger the payout, and every staker is slashed pro rata. No claims committee, no vote, no discretionary compensation.

This is the Soroban implementation of [Catalysis](https://catalysis.network), which is live and audited on Ethereum. The full design is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Built for Stellar Community Fund #46.

> **Status: pre-grant MVP.** The happy path runs end to end in tests and on testnet. The extensions scheduled for the grant milestones are listed in [Scope](#scope). The code is not audited — do not use it with real funds.

## How it works

```
 stakers ──stake XLM──►  CoverPool  ◄──reserve limit── Policy ◄──buy cover── vault
                            │                           │
                            │                   observe b_rate
                            │                           ▼
                            │                     Blend adapter ──get_reserve──► Blend pool
                            │
          payout to vault ◄─┘  (anyone calls Policy.settle once b_rate has fallen past the trigger)
```

| Contract | Role |
|---|---|
| [`cover_pool`](contracts/cover_pool/src/lib.rs) | Staked capital. Mints shares, reserves capacity for policies, distributes premium through a per-share accumulator, and pays claims by lowering the share value, so every staker is slashed pro rata in a constant number of writes. Fully collateralised: coverage written never exceeds capital, and capital backing live coverage cannot be unstaked. |
| [`policy`](contracts/policy/src/lib.rs) | Issues cover on a supply position in a Blend reserve. Records the reserve's `b_rate` at issuance, charges premium, and pays `covered_amount × decline` (capped at the limit) once the decline passes the trigger. `settle` and `expire` are permissionless. |
| [`blend_adapter`](contracts/blend_adapter/src/lib.rs) | Reads a reserve's `b_rate` through Blend V2's public `get_reserve`. Blend socializes bad debt by lowering `b_rate`, so a decline is the on-ledger signature of supplier loss. Read-only. |
| [`mock_blend_pool`](contracts/mock_blend_pool/src/lib.rs) | **Test and demo harness only.** Same `get_reserve` interface and `Reserve` layout as Blend V2, with a setter to replay a bad-debt event. |
| [`common`](contracts/common/src/lib.rs) | Cross-contract interfaces and the Blend V2 types, mirrored field-for-field from `blend-capital/blend-contracts-v2`. |

**Why XLM.** Native XLM has no issuer, so nobody can freeze or claw back the pool's capital. That matters on Stellar, where an issuer's freeze and clawback rights reach balances held by contracts (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §6 and [`data/issuer-flags/`](data/issuer-flags/)). The pool asset is a constructor parameter, so a USDC pool is a redeploy rather than a code change.

## Run the tests

```sh
cargo test --workspace
```

Unit tests cover staking, locking, capacity reservation, premium sharing, pro-rata slashing, and authorization. [`integration-tests/tests/happy_path.rs`](integration-tests/tests/happy_path.rs) runs all four contracts together:

1. Alice stakes 60,000 XLM and Bob stakes 40,000 XLM.
2. A vault with 50,000 XLM supplied to Blend buys 20,000 XLM of cover for a 500 XLM premium.
3. Bad debt is socialized and the reserve's `b_rate` drops 12%.
4. A third party settles, and the vault receives its 6,000 XLM loss.
5. Both stakers are slashed exactly 6%, keep their premium, and unstake.

## Run the demo on testnet

Needs the [Stellar CLI](https://developers.stellar.org/docs/tools/cli) and `jq`.

```sh
./scripts/demo_testnet.sh                # full run
DEMO_PAUSE=1 ./scripts/demo_testnet.sh   # pause between steps, for recording
```

The script builds the contracts, funds five accounts with Friendbot, deploys everything, and runs the same scenario at 1/10 scale. Contract IDs are written to `deployments/testnet.json`.

## Testnet deployments

| Contract | ID |
|---|---|
| CoverPool | _pending first deploy_ |
| Policy | _pending first deploy_ |
| Blend adapter | _pending first deploy_ |
| Mock Blend pool | _pending first deploy_ |

## Scope

Built now (pre-grant MVP): everything above.

Scheduled for the grant milestones, per [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §11:

- Integration with a real Blend V2 pool on testnet, plus reading pending bad debt from the backstop
- Withdrawal notice period, and a claim freeze keyed to pending bad debt
- DeFindex adapter
- Oracle-derived triggers with a cross-feed divergence gate
- Keeper-driven TTL maintenance
- USDC pools
- Audit, then mainnet

## License

[Apache-2.0](LICENSE).

*Contract code, tests and documentation in this repository were written with AI assistance and reviewed by the Catalysis team, who own every design decision in it.*
