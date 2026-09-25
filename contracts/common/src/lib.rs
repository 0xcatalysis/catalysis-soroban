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
// CoverPool, as seen by the CoveredVault
// ---------------------------------------------------------------------------

#[contractclient(name = "CoverPoolClient")]
pub trait CoverPoolInterface {
    /// Reserves `limit` of pool capital for `vault` and pulls the premium
    /// from `payer`. Calling again for the same vault adds to the reservation.
    /// Returns the premium charged.
    fn open_cover(env: Env, vault: Address, payer: Address, limit: i128) -> i128;
    /// Pays `amount` of the vault's remaining cover to the vault, slashing
    /// every staker pro rata. Returns the amount paid.
    fn claim(env: Env, vault: Address, amount: i128) -> i128;
    /// Releases whatever of the vault's cover remains unclaimed.
    fn close_cover(env: Env, vault: Address);
}

// ---------------------------------------------------------------------------
// Strategies, as seen by the CoveredVault
// ---------------------------------------------------------------------------

#[contractclient(name = "StrategyClient")]
pub trait StrategyInterface {
    /// Current value of the strategy's position, in the vault asset.
    fn total_value(env: Env) -> i128;
    /// Moves `amount` of the vault asset back to the vault.
    fn withdraw(env: Env, amount: i128);
}

// ---------------------------------------------------------------------------
// Blend V2 pool types, mirrored field-for-field from
// blend-capital/blend-contracts-v2 (pool/src/storage.rs, pool/src/pool/reserve.rs)
// so the milestone-1 Blend strategy can read a real Blend pool through its
// public `get_reserve`.
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
