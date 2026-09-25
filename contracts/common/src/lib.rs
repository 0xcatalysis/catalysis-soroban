//! Shared types and cross-contract interfaces.
//!
//! Contracts call each other through the `#[contractclient]` traits defined
//! here rather than by depending on each other's crates, so no contract's
//! exported functions leak into another contract's Wasm.

#![no_std]

use soroban_sdk::{contractclient, contracttype, Address, Env};

/// Basis-point denominator.
pub const BPS: i128 = 10_000;

// ---------------------------------------------------------------------------
// CoverPool, as seen by the Policy contract
// ---------------------------------------------------------------------------

#[contractclient(name = "CoverPoolClient")]
pub trait CoverPoolInterface {
    /// Capital not yet committed to any policy.
    fn available_capacity(env: Env) -> i128;
    /// Locks `limit` of pool capital behind `policy_id`, payable to `beneficiary`.
    fn reserve(env: Env, policy_id: u64, limit: i128, beneficiary: Address);
    /// Returns a policy's reserved capacity to the pool (policy expired).
    fn release(env: Env, policy_id: u64);
    /// Pulls `amount` of premium from `from` and credits it to stakers.
    fn accrue_premium(env: Env, policy_id: u64, from: Address, amount: i128);
    /// Pays `payout` to the beneficiary recorded at reservation.
    fn settle_claim(env: Env, policy_id: u64, payout: i128) -> i128;
}

// ---------------------------------------------------------------------------
// Observation adapters, as seen by the Policy contract
// ---------------------------------------------------------------------------

#[contractclient(name = "AdapterClient")]
pub trait AdapterInterface {
    /// Returns the covered position's value rate. For Blend this is the
    /// reserve's `b_rate` (12 decimals): it falls when bad debt is socialized
    /// across suppliers.
    fn observe(env: Env, target: Address, asset: Address) -> i128;
}

// ---------------------------------------------------------------------------
// Blend V2 pool types, mirrored field-for-field from
// blend-capital/blend-contracts-v2 (pool/src/storage.rs, pool/src/pool/reserve.rs)
// so the adapter can read a real Blend pool through its public `get_reserve`.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveConfig {
    pub index: u32,
    pub decimals: u32,
    pub c_factor: u32,
    pub l_factor: u32,
    pub util: u32,
    pub max_util: u32,
    pub r_base: u32,
    pub r_one: u32,
    pub r_two: u32,
    pub r_three: u32,
    pub reactivity: u32,
    pub supply_cap: i128,
    pub enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReserveData {
    pub d_rate: i128,
    pub b_rate: i128,
    pub ir_mod: i128,
    pub b_supply: i128,
    pub d_supply: i128,
    pub backstop_credit: i128,
    pub last_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reserve {
    pub asset: Address,
    pub config: ReserveConfig,
    pub data: ReserveData,
    pub scalar: i128,
}

#[contractclient(name = "BlendPoolClient")]
pub trait BlendPoolInterface {
    fn get_reserve(env: Env, asset: Address) -> Reserve;
}
