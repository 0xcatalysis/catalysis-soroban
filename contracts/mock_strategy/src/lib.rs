//! MockStrategy — TEST AND DEMO ONLY.
//!
//! Stands in for a yield strategy (a Blend market in milestone 1). It holds
//! real XLM so that a simulated loss really moves tokens out and the balances
//! on the explorer match the demo story. There is no yield.
//!
//! The vault funds the strategy by transferring XLM straight to this contract's
//! address; there is no deposit function. `total_value` is simply the token
//! balance. The vault pulls funds back with `withdraw`. The admin triggers a
//! loss with `simulate_loss`, which sends part of the balance to the admin.
//!
//! Never deploy this contract with real funds.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, Env,
};

/// Basis-point denominator.
const BPS: u32 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyConfig {
    pub admin: Address,
    /// The only address allowed to withdraw.
    pub vault: Address,
    /// Stellar Asset Contract of the held asset (native XLM for the MVP).
    pub asset: Address,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Config,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    InvalidAmount = 2,
    InsufficientBalance = 3,
    InvalidBps = 4,
}

#[contractevent]
pub struct Withdrawn {
    pub amount: i128,
    pub value_after: i128,
}

/// TEST AND DEMO ONLY: the admin removed `amount` to simulate a loss.
#[contractevent]
pub struct LossSimulated {
    pub bps: u32,
    pub amount: i128,
    pub value_after: i128,
}

#[contract]
pub struct MockStrategy;

#[contractimpl]
impl MockStrategy {
    pub fn __constructor(env: Env, admin: Address, vault: Address, asset: Address) {
        env.storage().instance().set(
            &DataKey::Config,
            &StrategyConfig {
                admin,
                vault,
                asset,
            },
        );
    }

    /// Current value of the position: the strategy's token balance.
    pub fn total_value(env: Env) -> i128 {
        balance(&env, &config(&env))
    }

    /// Moves `amount` back to the vault. The vault is the direct invoker, so
    /// its authorization is implicit.
    pub fn withdraw(env: Env, amount: i128) {
        let cfg = config(&env);
        cfg.vault.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if amount > balance(&env, &cfg) {
            panic_with_error!(&env, Error::InsufficientBalance);
        }
        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &cfg.vault,
            &amount,
        );
        Withdrawn {
            amount,
            value_after: balance(&env, &cfg),
        }
        .publish(&env);
    }

    /// TEST AND DEMO ONLY. Transfers `bps` of the current balance to the
    /// admin, so the strategy is worth less. Admin only.
    pub fn simulate_loss(env: Env, bps: u32) -> i128 {
        let cfg = config(&env);
        cfg.admin.require_auth();
        if bps == 0 || bps > BPS {
            panic_with_error!(&env, Error::InvalidBps);
        }
        let amount = balance(&env, &cfg) * bps as i128 / BPS as i128; // rounds down
        if amount > 0 {
            token::Client::new(&env, &cfg.asset).transfer(
                &env.current_contract_address(),
                &cfg.admin,
                &amount,
            );
        }
        LossSimulated {
            bps,
            amount,
            value_after: balance(&env, &cfg),
        }
        .publish(&env);
        amount
    }

    pub fn config(env: Env) -> StrategyConfig {
        config(&env)
    }
}

fn config(env: &Env) -> StrategyConfig {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn balance(env: &Env, cfg: &StrategyConfig) -> i128 {
    token::Client::new(env, &cfg.asset).balance(&env.current_contract_address())
}

#[cfg(test)]
mod test;
