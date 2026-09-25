# Catalysis on Soroban

Covered vaults for Stellar. A vault takes XLM deposits and allocates them to strategies (Blend markets, mocked for now). A staked XLM insurance pool (CoverPool) pays any shortfall when a depositor withdraws, slashing its stakers pro rata. This is the Soroban port of Catalysis, which is live and audited on Ethereum. It is being built for a Stellar Community Fund #46 grant application (deadline 8 November 2026).

**Start here:** `docs/PLAN.md` is the current build plan and task list. `docs/ARCHITECTURE.md` is the grant's technical architecture document and must always match the code.

## Rules

- **Happy path only.** Make the clean demo path run end to end. Do not build for edge cases or unlikely scenarios. When you notice one, add it to the known-limitations list in `docs/PLAN.md` §7 and move on.
- **Native XLM only.** No USDC, RWAs or other issued assets in the MVP.
- **Mocks are labelled.** Anything that stands in for a real protocol says "TEST AND DEMO ONLY" in its doc comment, the README and the architecture doc.
- **Docs match code.** Never describe behaviour the code does not have. Milestone work is marked as milestone work.
- **Stay inside this repo.** The parent folder (`stellar/`) holds the founders' grant application files. Do not edit them.
- **Don't change decisions** listed in `docs/PLAN.md` §2 without asking the founders.

## Commands

```sh
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
stellar contract build                      # WASMs -> target/wasm32v1-none/release/
./scripts/demo_testnet.sh                   # end-to-end demo on testnet
DEMO_PAUSE=1 ./scripts/demo_testnet.sh      # step mode, for recording the video
./data/issuer-flags/check_issuer_flags.sh   # re-check issuer flags on the live ledger
```

Needs Rust stable with the `wasm32v1-none` target, the Stellar CLI and `jq`.

## Conventions

- **soroban-sdk 28.0.0**, no other dependencies. Contracts are `#![no_std]`, with `crate-type = ["cdylib", "rlib"]`.
- **Cross-contract calls go through `#[contractclient]` traits in `contracts/common`**, never by depending on another contract crate. Depending on a contract crate leaks its exports into your WASM. Integration tests may depend on contract crates directly.
- **Errors:** a `#[contracterror]` enum per contract plus `panic_with_error!`. **Events:** `#[contractevent]` structs with `.publish(&env)`.
- **Upgrades:** `env.deployer().update_current_contract(ContractExecutable::Wasm(hash))`. The old `update_current_contract_wasm` is deprecated in SDK 28.
- **Tests** use `env.mock_all_auths_allowing_non_root_auth()`. Nested token transfers authorized by a root signer need the non-root variant.
- **Amounts** are in stroops (1 XLM = 10,000,000). Share and value maths round down, in the contract's favour.
- **Storage.** Config and aggregate state go in instance storage; per-user entries go in persistent storage and have their TTL extended on write. `test_snapshots/` is gitignored.
- **CI** (`.github/workflows/ci.yml`) runs fmt, clippy `-D warnings`, the tests, and a `wasm32v1-none` release build. Keep it green.
