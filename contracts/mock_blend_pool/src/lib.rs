//! Mock Blend V2 pool — TEST AND DEMO HARNESS ONLY.
//!
//! Exposes the same `get_reserve` signature and `Reserve` layout as a real
//! Blend V2 pool, with an admin setter for `b_rate` so tests and the testnet
//! demo can replay a bad-debt socialization event (such as the February 2026
//! YieldBlox loss) on demand. Never used in production: in production the
//! adapter reads a real Blend pool.

#![no_std]

use catalysis_common::{Reserve, ReserveConfig, ReserveData};
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

/// 1.0 with Blend's 12-decimal rate precision.
pub const RATE_ONE: i128 = 1_000_000_000_000;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    BRate(Address),
}

#[contract]
pub struct MockBlendPool;

#[contractimpl]
impl MockBlendPool {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Sets the reserve's `b_rate`. Lowering it simulates Blend socializing
    /// bad debt across that reserve's suppliers.
    pub fn set_b_rate(env: Env, asset: Address, b_rate: i128) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::BRate(asset), &b_rate);
    }

    pub fn get_reserve(env: Env, asset: Address) -> Reserve {
        let b_rate: i128 = env
            .storage()
            .instance()
            .get(&DataKey::BRate(asset.clone()))
            .unwrap_or(RATE_ONE);
        Reserve {
            asset,
            config: ReserveConfig {
                index: 0,
                decimals: 7,
                c_factor: 9_000_000,
                l_factor: 9_000_000,
                util: 8_000_000,
                max_util: 9_500_000,
                r_base: 100_000,
                r_one: 500_000,
                r_two: 5_000_000,
                r_three: 15_000_000,
                reactivity: 200,
                supply_cap: i128::MAX,
                enabled: true,
            },
            data: ReserveData {
                d_rate: RATE_ONE,
                b_rate,
                ir_mod: 10_000_000,
                b_supply: 0,
                d_supply: 0,
                backstop_credit: 0,
                last_time: env.ledger().timestamp(),
            },
            scalar: 10_000_000,
        }
    }
}
