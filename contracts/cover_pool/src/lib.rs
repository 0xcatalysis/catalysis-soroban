//! CoverPool — a staked capital pool that insures covered vaults.
//!
//! Stakers deposit a Stellar asset (XLM for the MVP) and receive shares. A
//! whitelisted CoveredVault opens cover on itself, paying a premium set by the
//! pool, and later claims against that cover, in as many partial claims as it
//! has withdrawals with a shortfall. Each claim is taken from pool capital, so
//! every staker is slashed pro rata through the share value; nobody's share
//! count changes. Premium is distributed to stakers through a per-share
//! accumulator.
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
/// Basis-point denominator.
const BPS: i128 = 10_000;

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
    /// Premium charged when cover is opened, in basis points of the limit.
    pub premium_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PoolState {
    /// Asset units backing all shares.
    pub total_capital: i128,
    pub total_shares: i128,
    /// Sum of every vault's remaining cover.
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

/// A vault's cover. `remaining` is what can still be claimed; `claimed` is
/// the running total already paid out.
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cover {
    pub remaining: i128,
    pub claimed: i128,
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
    /// Whitelisted vault. Instance storage: the set is small and admin-managed.
    Vault(Address),
    Position(Address),
    Cover(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    NotWhitelisted = 2,
    InvalidAmount = 3,
    InsufficientCapacity = 4,
    StillLocked = 5,
    InsufficientShares = 6,
    CapitalBackingCoverage = 7,
    NoCover = 8,
    ClaimExceedsCover = 9,
    VaultAlreadyAdded = 10,
    /// Shares exist but capital is zero: the pool was fully slashed, so
    /// no share price exists to mint at.
    PoolDrained = 11,
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
pub struct VaultAdded {
    #[topic]
    pub vault: Address,
}

#[contractevent]
pub struct CoverOpened {
    #[topic]
    pub vault: Address,
    pub limit: i128,
    pub premium: i128,
    pub remaining_after: i128,
}

#[contractevent]
pub struct CoverClosed {
    #[topic]
    pub vault: Address,
    pub released: i128,
}

/// Emitted when a claim slashes the pool.
#[contractevent]
pub struct Slashed {
    #[topic]
    pub vault: Address,
    pub amount: i128,
    pub remaining_after: i128,
    pub total_capital_after: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CoverPool;

#[contractimpl]
impl CoverPool {
    pub fn __constructor(
        env: Env,
        admin: Address,
        asset: Address,
        lock_ledgers: u32,
        premium_bps: u32,
    ) {
        env.storage().instance().set(
            &DataKey::Config,
            &PoolConfig {
                admin,
                asset,
                lock_ledgers,
                premium_bps,
            },
        );
        env.storage()
            .instance()
            .set(&DataKey::State, &PoolState::default());
    }

    /// Whitelists a CoveredVault. Only whitelisted vaults can open cover on
    /// this pool. Admin only.
    pub fn add_vault(env: Env, vault: Address) {
        config(&env).admin.require_auth();
        let key = DataKey::Vault(vault.clone());
        if env.storage().instance().has(&key) {
            panic_with_error!(&env, Error::VaultAlreadyAdded);
        }
        env.storage().instance().set(&key, &true);
        VaultAdded { vault }.publish(&env);
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
        if st.total_shares > 0 && st.total_capital == 0 {
            panic_with_error!(&env, Error::PoolDrained);
        }

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
        if amount == 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
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

    // ----- vault-facing ----------------------------------------------------

    /// Capital not yet committed to any vault's cover.
    pub fn available_capacity(env: Env) -> i128 {
        let st = state(&env);
        st.total_capital - st.coverage_written
    }

    /// Reserves `limit` of pool capital for `vault` and charges the premium
    /// (`limit * premium_bps / 10_000`) from `payer`, crediting it to stakers.
    /// Calling again for the same vault adds to its remaining cover. Returns
    /// the premium charged.
    ///
    /// The vault is the caller; its authorization is implicit on the direct
    /// call. `payer` authorizes the premium transfer through the token.
    pub fn open_cover(env: Env, vault: Address, payer: Address, limit: i128) -> i128 {
        require_vault(&env, &vault);
        if limit <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cfg = config(&env);
        let mut st = state(&env);
        if st.coverage_written + limit > st.total_capital {
            panic_with_error!(&env, Error::InsufficientCapacity);
        }

        let premium = limit * cfg.premium_bps as i128 / BPS;
        if premium > 0 {
            token::Client::new(&env, &cfg.asset).transfer(
                &payer,
                env.current_contract_address(),
                &premium,
            );
            // total_capital > 0 here, so shares exist to credit.
            st.premium_index += premium * SCALE / st.total_shares;
            st.premiums_paid += premium;
        }

        let mut cover = cover(&env, &vault);
        cover.remaining += limit;
        st.coverage_written += limit;
        save_cover(&env, &vault, &cover);
        save_state(&env, &st);
        CoverOpened {
            vault,
            limit,
            premium,
            remaining_after: cover.remaining,
        }
        .publish(&env);
        premium
    }

    /// Pays `amount` of the vault's remaining cover to the vault. This is the
    /// slashing moment: capital leaves the pool and every staker's share
    /// value falls pro rata. Four writes regardless of staker count: pool
    /// state, the cover entry, and the two token balances. Returns the amount.
    pub fn claim(env: Env, vault: Address, amount: i128) -> i128 {
        require_vault(&env, &vault);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cfg = config(&env);
        let mut cover = cover(&env, &vault);
        if cover.remaining == 0 {
            panic_with_error!(&env, Error::NoCover);
        }
        if amount > cover.remaining {
            panic_with_error!(&env, Error::ClaimExceedsCover);
        }

        let mut st = state(&env);
        st.total_capital -= amount; // share value falls for every staker
        st.coverage_written -= amount;
        st.claims_paid += amount;
        cover.remaining -= amount;
        cover.claimed += amount;
        save_cover(&env, &vault, &cover);
        save_state(&env, &st);

        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &vault,
            &amount,
        );
        Slashed {
            vault,
            amount,
            remaining_after: cover.remaining,
            total_capital_after: st.total_capital,
        }
        .publish(&env);
        amount
    }

    /// Releases whatever of the vault's cover remains unclaimed. The claimed
    /// total is kept for the record.
    pub fn close_cover(env: Env, vault: Address) {
        require_vault(&env, &vault);
        let mut cover = cover(&env, &vault);
        let released = cover.remaining;
        let mut st = state(&env);
        st.coverage_written -= released;
        cover.remaining = 0;
        save_cover(&env, &vault, &cover);
        save_state(&env, &st);
        CoverClosed { vault, released }.publish(&env);
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

    /// A vault's cover. Zero for a vault that never opened any.
    pub fn cover(env: Env, vault: Address) -> Cover {
        cover(&env, &vault)
    }

    pub fn is_vault(env: Env, vault: Address) -> bool {
        env.storage().instance().has(&DataKey::Vault(vault))
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

/// The vault must have signed (implicit when it is the direct invoker) and be
/// whitelisted.
fn require_vault(env: &Env, vault: &Address) {
    vault.require_auth();
    if !env.storage().instance().has(&DataKey::Vault(vault.clone())) {
        panic_with_error!(env, Error::NotWhitelisted);
    }
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

fn cover(env: &Env, vault: &Address) -> Cover {
    env.storage()
        .persistent()
        .get(&DataKey::Cover(vault.clone()))
        .unwrap_or_default()
}

fn save_cover(env: &Env, vault: &Address, cover: &Cover) {
    let key = DataKey::Cover(vault.clone());
    env.storage().persistent().set(&key, cover);
    bump(env, &key);
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
