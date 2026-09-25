//! Blend V2 observation adapter.
//!
//! Reads a reserve's `b_rate` — the bToken-to-underlying conversion rate —
//! through the pool's public `get_reserve`. Blend socializes residual bad debt
//! by lowering `b_rate` (`default_liabilities`), so a decline is the on-ledger
//! signature of supplier loss. Read-only: this contract calls no
//! state-changing Blend function and holds no authority over the pool.

#![no_std]

use catalysis_common::BlendPoolClient;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[contract]
pub struct BlendAdapter;

#[contractimpl]
impl BlendAdapter {
    /// `target` is a Blend V2 pool; `asset` is the reserve's underlying asset.
    /// Returns `b_rate` with 12 decimals (1.0 = 1_000_000_000_000).
    pub fn observe(env: Env, target: Address, asset: Address) -> i128 {
        BlendPoolClient::new(&env, &target)
            .get_reserve(&asset)
            .data
            .b_rate
    }
}
