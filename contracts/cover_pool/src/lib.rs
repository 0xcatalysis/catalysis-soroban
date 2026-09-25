//! CoverPool — a staked capital pool that underwrites coverage.
//!
//! Stakers deposit a Stellar asset (XLM for the MVP) and receive shares. The
//! Policy contract reserves pool capital behind each policy it issues. When a
//! policy pays out, the payout is taken from pool capital and every staker is
//! slashed pro rata through the share value; nobody's share count changes.
//! Premium is distributed to stakers through a per-share accumulator.
//!
//! Pools are fully collateralised: coverage written can never exceed capital,
//! and capital backing live coverage cannot be withdrawn.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, Env,
};

/// Fixed-point scale for the premium-per-share accumulator.
const SCALE: i128 = 1_000_000_000_000_000_000; // 1e18

/// Persistent-entry TTL maintenance (ledgers). ~30 days at 5s ledgers.
const TTL_THRESHOLD: u32 = 17_280 * 15;
const TTL_EXTEND_TO: u32 = 17_280 * 30;

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolConfig {
    pub admin: Address,
    /// Stellar Asset Contract of the staked asset (native XLM for the MVP).
    pub asset: Address,
    /// Minimum stake duration before withdrawal, in ledgers.
    pub lock_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolState {
    /// Asset units backing all shares.
    pub total_capital: i128,
    pub total_shares: i128,
    /// Sum of reserved policy limits.
    pub coverage_written: i128,
    /// Cumulative premium per share, scaled by 1e18.
    pub premium_index: i128,
    pub premiums_paid: i128,
    pub claims_paid: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    pub shares: i128,
    pub premium_index_entry: i128,
    pub premium_owed: i128,
    pub locked_until: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reservation {
    pub limit: i128,
    pub beneficiary: Address,
}

/// What a staker's position is worth right now.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionView {
    pub shares: i128,
    pub value: i128,
    pub premium_claimable: i128,
    pub locked_until: u32,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Config,
    State,
    PolicyContract,
    Position(Address),
    Reservation(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    PolicyContractAlreadySet = 2,
    InvalidAmount = 3,
    InsufficientCapacity = 4,
    StillLocked = 5,
    InsufficientShares = 6,
    CapitalBackingCoverage = 7,
    UnknownReservation = 8,
    PayoutExceedsLimit = 9,
    ReservationExists = 10,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[contractevent]
pub struct Staked {
    #[topic]
    pub staker: Address,
    pub amount: i128,
    pub shares: i128,
}

#[contractevent]
pub struct Unstaked {
    #[topic]
    pub staker: Address,
    pub amount: i128,
    pub shares: i128,
}

#[contractevent]
pub struct CoverageReserved {
    #[topic]
    pub policy_id: u64,
    pub limit: i128,
}

#[contractevent]
pub struct CoverageReleased {
    #[topic]
    pub policy_id: u64,
    pub limit: i128,
}

#[contractevent]
pub struct PremiumAccrued {
    #[topic]
    pub policy_id: u64,
    pub amount: i128,
}

/// Emitted when a payout slashes the pool.
#[contractevent]
pub struct Slashed {
    #[topic]
    pub policy_id: u64,
    #[topic]
    pub beneficiary: Address,
    pub payout: i128,
    pub total_capital_after: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CoverPool;

#[contractimpl]
impl CoverPool {
    pub fn __constructor(env: Env, admin: Address, asset: Address, lock_ledgers: u32) {
        env.storage().instance().set(
            &DataKey::Config,
            &PoolConfig {
                admin,
                asset,
                lock_ledgers,
            },
        );
        env.storage()
            .instance()
            .set(&DataKey::State, &PoolState::default());
    }

    /// One-time wiring of the Policy contract, the only address allowed to
    /// reserve, release, accrue premium on or settle against this pool.
    pub fn set_policy_contract(env: Env, policy_contract: Address) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        if env.storage().instance().has(&DataKey::PolicyContract) {
            panic_with_error!(&env, Error::PolicyContractAlreadySet);
        }
        env.storage()
            .instance()
            .set(&DataKey::PolicyContract, &policy_contract);
    }

    // ----- staking ---------------------------------------------------------

    /// Deposits `amount` of the pool asset. Returns shares minted.
    pub fn stake(env: Env, from: Address, amount: i128) -> i128 {
        from.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cfg = config(&env);
        let mut st = state(&env);

        // Custody-lock: capital moves into the pool's own balance.
        token::Client::new(&env, &cfg.asset).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );

        let shares = if st.total_shares == 0 {
            amount
        } else {
            amount * st.total_shares / st.total_capital // rounds down, pool's favour
        };

        let mut pos = position_or_new(&env, &from, &st);
        settle_premium(&mut pos, &st);
        pos.shares += shares;
        pos.locked_until = env.ledger().sequence() + cfg.lock_ledgers;

        st.total_capital += amount;
        st.total_shares += shares;

        save_position(&env, &from, &pos);
        save_state(&env, &st);
        Staked {
            staker: from,
            amount,
            shares,
        }
        .publish(&env);
        shares
    }

    /// Burns `shares` and returns their current value. Only capital not
    /// backing live coverage can leave the pool.
    pub fn unstake(env: Env, from: Address, shares: i128) -> i128 {
        from.require_auth();
        let cfg = config(&env);
        let mut st = state(&env);
        let mut pos = position(&env, &from);

        if shares <= 0 || shares > pos.shares {
            panic_with_error!(&env, Error::InsufficientShares);
        }
        if env.ledger().sequence() < pos.locked_until {
            panic_with_error!(&env, Error::StillLocked);
        }

        let amount = shares * st.total_capital / st.total_shares; // rounds down
        if st.total_capital - amount < st.coverage_written {
            panic_with_error!(&env, Error::CapitalBackingCoverage);
        }

        settle_premium(&mut pos, &st);
        pos.shares -= shares;
        st.total_capital -= amount;
        st.total_shares -= shares;

        save_position(&env, &from, &pos);
        save_state(&env, &st);
        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &from,
            &amount,
        );
        Unstaked {
            staker: from,
            amount,
            shares,
        }
        .publish(&env);
        amount
    }

    /// Pays out all premium earned by `from`.
    pub fn claim_premium(env: Env, from: Address) -> i128 {
        from.require_auth();
        let cfg = config(&env);
        let st = state(&env);
        let mut pos = position(&env, &from);
        settle_premium(&mut pos, &st);
        let amount = pos.premium_owed;
        pos.premium_owed = 0;
        save_position(&env, &from, &pos);
        if amount > 0 {
            token::Client::new(&env, &cfg.asset).transfer(
                &env.current_contract_address(),
                &from,
                &amount,
            );
        }
        amount
    }

    // ----- policy-facing ---------------------------------------------------

    pub fn available_capacity(env: Env) -> i128 {
        let st = state(&env);
        st.total_capital - st.coverage_written
    }

    /// Reserves `limit` of capital behind a policy. The beneficiary is fixed
    /// here, so no later call can redirect a payout.
    pub fn reserve(env: Env, policy_id: u64, limit: i128, beneficiary: Address) {
        require_policy_contract(&env);
        if limit <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let key = DataKey::Reservation(policy_id);
        if env.storage().persistent().has(&key) {
            panic_with_error!(&env, Error::ReservationExists);
        }
        let mut st = state(&env);
        if st.coverage_written + limit > st.total_capital {
            panic_with_error!(&env, Error::InsufficientCapacity);
        }
        st.coverage_written += limit;
        env.storage()
            .persistent()
            .set(&key, &Reservation { limit, beneficiary });
        bump(&env, &key);
        save_state(&env, &st);
        CoverageReserved { policy_id, limit }.publish(&env);
    }

    /// Returns a policy's reserved capital to the pool.
    pub fn release(env: Env, policy_id: u64) {
        require_policy_contract(&env);
        let res = reservation(&env, policy_id);
        let mut st = state(&env);
        st.coverage_written -= res.limit;
        env.storage()
            .persistent()
            .remove(&DataKey::Reservation(policy_id));
        save_state(&env, &st);
        CoverageReleased {
            policy_id,
            limit: res.limit,
        }
        .publish(&env);
    }

    /// Pulls premium from `from` and credits it to stakers pro rata.
    pub fn accrue_premium(env: Env, policy_id: u64, from: Address, amount: i128) {
        require_policy_contract(&env);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cfg = config(&env);
        let mut st = state(&env);
        token::Client::new(&env, &cfg.asset).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );
        st.premium_index += amount * SCALE / st.total_shares;
        st.premiums_paid += amount;
        save_state(&env, &st);
        PremiumAccrued { policy_id, amount }.publish(&env);
    }

    /// Pays `payout` to the beneficiary recorded at reservation and slashes
    /// every staker pro rata. Three writes regardless of staker count: pool
    /// state, and the pool's and beneficiary's token balances.
    pub fn settle_claim(env: Env, policy_id: u64, payout: i128) -> i128 {
        require_policy_contract(&env);
        let cfg = config(&env);
        let res = reservation(&env, policy_id);
        if payout <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if payout > res.limit {
            panic_with_error!(&env, Error::PayoutExceedsLimit);
        }

        let mut st = state(&env);
        st.total_capital -= payout; // share value falls for every staker
        st.coverage_written -= res.limit;
        st.claims_paid += payout;
        env.storage()
            .persistent()
            .remove(&DataKey::Reservation(policy_id));
        save_state(&env, &st);

        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &res.beneficiary,
            &payout,
        );
        Slashed {
            policy_id,
            beneficiary: res.beneficiary,
            payout,
            total_capital_after: st.total_capital,
        }
        .publish(&env);
        payout
    }

    // ----- views -----------------------------------------------------------

    pub fn config(env: Env) -> PoolConfig {
        config(&env)
    }

    pub fn state(env: Env) -> PoolState {
        state(&env)
    }

    pub fn position(env: Env, staker: Address) -> PositionView {
        let st = state(&env);
        let mut pos = position(&env, &staker);
        settle_premium(&mut pos, &st);
        let value = if st.total_shares == 0 {
            0
        } else {
            pos.shares * st.total_capital / st.total_shares
        };
        PositionView {
            shares: pos.shares,
            value,
            premium_claimable: pos.premium_owed,
            locked_until: pos.locked_until,
        }
    }

    pub fn reservation(env: Env, policy_id: u64) -> Reservation {
        reservation(&env, policy_id)
    }

    /// Value of one share, scaled by 1e7 (1.0 = 10_000_000).
    pub fn share_value(env: Env) -> i128 {
        let st = state(&env);
        if st.total_shares == 0 {
            return 10_000_000;
        }
        st.total_capital * 10_000_000 / st.total_shares
    }

    // ----- operations ------------------------------------------------------

    /// Permissionless TTL maintenance for staker positions.
    pub fn bump_position(env: Env, staker: Address) {
        bump(&env, &DataKey::Position(staker));
    }

    /// Replaces the contract Wasm. Admin only.
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
        config(&env).admin.require_auth();
        env.deployer()
            .update_current_contract(soroban_sdk::ContractExecutable::Wasm(new_wasm_hash));
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn config(env: &Env) -> PoolConfig {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn state(env: &Env) -> PoolState {
    env.storage()
        .instance()
        .get(&DataKey::State)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn save_state(env: &Env, st: &PoolState) {
    env.storage().instance().set(&DataKey::State, st);
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn require_policy_contract(env: &Env) {
    let policy: Address = env
        .storage()
        .instance()
        .get(&DataKey::PolicyContract)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized));
    policy.require_auth();
}

fn position(env: &Env, who: &Address) -> Position {
    env.storage()
        .persistent()
        .get(&DataKey::Position(who.clone()))
        .unwrap_or_else(|| panic_with_error!(env, Error::InsufficientShares))
}

fn position_or_new(env: &Env, who: &Address, st: &PoolState) -> Position {
    env.storage()
        .persistent()
        .get(&DataKey::Position(who.clone()))
        .unwrap_or(Position {
            shares: 0,
            premium_index_entry: st.premium_index,
            premium_owed: 0,
            locked_until: 0,
        })
}

fn save_position(env: &Env, who: &Address, pos: &Position) {
    let key = DataKey::Position(who.clone());
    env.storage().persistent().set(&key, pos);
    bump(env, &key);
}

fn reservation(env: &Env, policy_id: u64) -> Reservation {
    env.storage()
        .persistent()
        .get(&DataKey::Reservation(policy_id))
        .unwrap_or_else(|| panic_with_error!(env, Error::UnknownReservation))
}

/// Moves premium earned since the position's last checkpoint into `premium_owed`.
fn settle_premium(pos: &mut Position, st: &PoolState) {
    pos.premium_owed += pos.shares * (st.premium_index - pos.premium_index_entry) / SCALE;
    pos.premium_index_entry = st.premium_index;
}

fn bump(env: &Env, key: &DataKey) {
    if env.storage().persistent().has(key) {
        env.storage()
            .persistent()
            .extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND_TO);
    }
}

#[cfg(test)]
mod test;
