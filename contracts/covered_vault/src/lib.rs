//! CoveredVault — an XLM vault whose depositors are insured by a CoverPool.
//!
//! Depositors receive internal, non-transferable shares. The curator (admin)
//! allocates idle XLM to whitelisted strategies and buys cover from the pool.
//! The vault tracks each depositor's principal. At withdrawal it compares the
//! principal being withdrawn with what the shares are worth; if the shares are
//! worth less, the vault claims the shortfall from the CoverPool in the same
//! transaction and the depositor is paid in full.
//!
//! There is no trigger, no oracle and no claims process: the shortfall is the
//! vault's own accounting. This matches Catalysis on Ethereum, where the
//! covered vault wrapper files its claim inside the withdrawal.

#![no_std]

use catalysis_common::{CoverPoolClient, StrategyClient, BPS};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, token,
    Address, Env, Vec,
};

/// Share of a shortfall the depositor bears, in basis points. Kept so the
/// withdrawal maths has the same shape as the Ethereum wrapper's deductible.
/// Zero in the MVP (founders' decision); not configurable.
pub const DEDUCTIBLE_BPS: i128 = 0;

/// Persistent-entry TTL maintenance (ledgers). ~30 days at 5s ledgers.
const TTL_THRESHOLD: u32 = 17_280 * 15;
const TTL_EXTEND_TO: u32 = 17_280 * 30;

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultConfig {
    /// The curator: allocates, adds strategies, buys cover.
    pub admin: Address,
    /// Stellar Asset Contract of the vault asset (native XLM for the MVP).
    pub asset: Address,
    pub cover_pool: Address,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VaultState {
    pub total_shares: i128,
    /// Asset held by the vault itself, not allocated to any strategy. This is
    /// accounting state, not the live token balance: a shortfall claimed from
    /// the pool passes through the vault's balance without touching it.
    pub idle: i128,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Position {
    pub shares: i128,
    /// Cost basis: deposits, reduced pro rata on withdrawal.
    pub principal: i128,
}

/// What a depositor's position is worth right now.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PositionView {
    pub shares: i128,
    /// What the shares are worth at the current NAV.
    pub value: i128,
    /// What the depositor is protected for.
    pub principal: i128,
    /// `principal - value` when positive: what the pool would pay today.
    pub shortfall: i128,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Config,
    State,
    Strategies,
    Position(Address),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    InvalidAmount = 2,
    InsufficientShares = 3,
    StrategyNotWhitelisted = 4,
    StrategyAlreadyAdded = 5,
    InsufficientIdle = 6,
    InsufficientLiquidity = 7,
    /// The deposit is too small to buy a single share at the current NAV.
    ZeroShares = 8,
    /// Shares exist but the vault is worth nothing, so they cannot be priced.
    VaultDrained = 9,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[contractevent]
pub struct StrategyAdded {
    #[topic]
    pub strategy: Address,
}

#[contractevent]
pub struct CoverBought {
    pub limit: i128,
    pub premium: i128,
}

#[contractevent]
pub struct CoverClosed {}

#[contractevent]
pub struct Deposited {
    #[topic]
    pub depositor: Address,
    pub amount: i128,
    pub shares: i128,
}

#[contractevent]
pub struct Allocated {
    #[topic]
    pub strategy: Address,
    pub amount: i128,
}

#[contractevent]
pub struct Withdrawn {
    #[topic]
    pub depositor: Address,
    pub shares: i128,
    /// What the shares were worth.
    pub value: i128,
    /// What the pool paid on top.
    pub shortfall_paid: i128,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CoveredVault;

#[contractimpl]
impl CoveredVault {
    pub fn __constructor(env: Env, admin: Address, asset: Address, cover_pool: Address) {
        env.storage().instance().set(
            &DataKey::Config,
            &VaultConfig {
                admin,
                asset,
                cover_pool,
            },
        );
        env.storage()
            .instance()
            .set(&DataKey::State, &VaultState::default());
        env.storage()
            .instance()
            .set(&DataKey::Strategies, &Vec::<Address>::new(&env));
    }

    // ----- curator ---------------------------------------------------------

    /// Whitelists a strategy the curator may allocate to. Admin only.
    pub fn add_strategy(env: Env, strategy: Address) {
        config(&env).admin.require_auth();
        let mut list = strategies(&env);
        if list.contains(&strategy) {
            panic_with_error!(&env, Error::StrategyAlreadyAdded);
        }
        list.push_back(strategy.clone());
        env.storage().instance().set(&DataKey::Strategies, &list);
        StrategyAdded { strategy }.publish(&env);
    }

    /// Buys `limit` of cover for this vault from the CoverPool. The admin
    /// pays the premium; their signature on this call covers the nested
    /// premium transfer. Returns the premium paid.
    pub fn buy_cover(env: Env, limit: i128) -> i128 {
        let cfg = config(&env);
        cfg.admin.require_auth();
        let premium = CoverPoolClient::new(&env, &cfg.cover_pool).open_cover(
            &env.current_contract_address(),
            &cfg.admin,
            &limit,
        );
        CoverBought { limit, premium }.publish(&env);
        premium
    }

    /// Releases whatever of this vault's cover remains unclaimed. Admin only.
    /// The pool requires the vault's own signature, which is implicit on this
    /// direct call; a plain account cannot call the pool for the vault.
    pub fn close_cover(env: Env) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        CoverPoolClient::new(&env, &cfg.cover_pool).close_cover(&env.current_contract_address());
        CoverClosed {}.publish(&env);
    }

    /// Moves `amount` of idle asset into a whitelisted strategy. Admin only.
    /// The vault is the direct invoker of the token, so no extra auth is
    /// needed for the transfer.
    pub fn allocate(env: Env, strategy: Address, amount: i128) {
        let cfg = config(&env);
        cfg.admin.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if !strategies(&env).contains(&strategy) {
            panic_with_error!(&env, Error::StrategyNotWhitelisted);
        }
        let mut st = state(&env);
        if amount > st.idle {
            panic_with_error!(&env, Error::InsufficientIdle);
        }
        st.idle -= amount;
        save_state(&env, &st);
        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &strategy,
            &amount,
        );
        Allocated { strategy, amount }.publish(&env);
    }

    // ----- depositors ------------------------------------------------------

    /// Deposits `amount` of the vault asset. Returns shares minted.
    pub fn deposit(env: Env, from: Address, amount: i128) -> i128 {
        from.require_auth();
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let cfg = config(&env);
        let mut st = state(&env);

        // Price the shares at the NAV before this deposit. `nav` reads the
        // strategies, not the vault's live balance, so the transfer below
        // cannot skew it.
        let shares = if st.total_shares == 0 {
            amount
        } else {
            let nav = nav(&env, &st);
            if nav == 0 {
                panic_with_error!(&env, Error::VaultDrained);
            }
            amount * st.total_shares / nav // rounds down, vault's favour
        };
        if shares == 0 {
            panic_with_error!(&env, Error::ZeroShares);
        }

        token::Client::new(&env, &cfg.asset).transfer(
            &from,
            env.current_contract_address(),
            &amount,
        );

        let mut pos = position(&env, &from);
        pos.shares += shares;
        pos.principal += amount;
        st.total_shares += shares;
        st.idle += amount;

        save_position(&env, &from, &pos);
        save_state(&env, &st);
        Deposited {
            depositor: from,
            amount,
            shares,
        }
        .publish(&env);
        shares
    }

    /// Burns `shares` and pays the depositor what they are worth plus, if that
    /// is less than the principal those shares represent, the shortfall
    /// claimed from the CoverPool. Returns the total paid.
    pub fn withdraw(env: Env, from: Address, shares: i128) -> i128 {
        from.require_auth();
        let cfg = config(&env);
        let mut st = state(&env);
        let mut pos = position(&env, &from);
        if shares <= 0 || shares > pos.shares {
            panic_with_error!(&env, Error::InsufficientShares);
        }

        // 1. Value the shares at today's NAV, before any liquidity moves.
        let value = shares * nav(&env, &st) / st.total_shares; // rounds down
        let protected = pos.principal * shares / pos.shares; // rounds down
        let shortfall = (protected - value).max(0);
        let claimable = shortfall - shortfall * DEDUCTIBLE_BPS / BPS;
        let paid = value + claimable;
        if paid == 0 {
            // Dust: the shares are worth nothing and protect nothing.
            panic_with_error!(&env, Error::InvalidAmount);
        }

        // 2. Bring `value` of liquidity into the vault: idle first, then the
        //    strategies in whitelist order.
        let from_idle = value.min(st.idle);
        st.idle -= from_idle;
        let mut need = value - from_idle;
        for strategy in strategies(&env).iter() {
            if need == 0 {
                break;
            }
            let client = StrategyClient::new(&env, &strategy);
            let take = need.min(client.total_value());
            if take > 0 {
                client.withdraw(&take);
                need -= take;
            }
        }
        if need > 0 {
            panic_with_error!(&env, Error::InsufficientLiquidity);
        }

        // 3. Claim the shortfall. The pool pays the vault; the amount passes
        //    through the vault's balance without touching `idle`.
        if claimable > 0 {
            CoverPoolClient::new(&env, &cfg.cover_pool)
                .claim(&env.current_contract_address(), &claimable);
        }

        // 4. Update the books, then pay.
        pos.shares -= shares;
        pos.principal -= protected;
        st.total_shares -= shares;
        save_position(&env, &from, &pos);
        save_state(&env, &st);

        token::Client::new(&env, &cfg.asset).transfer(
            &env.current_contract_address(),
            &from,
            &paid,
        );
        Withdrawn {
            depositor: from,
            shares,
            value,
            shortfall_paid: claimable,
        }
        .publish(&env);
        paid
    }

    // ----- views -----------------------------------------------------------

    /// Idle asset plus what every strategy reports.
    pub fn nav(env: Env) -> i128 {
        nav(&env, &state(&env))
    }

    pub fn position(env: Env, depositor: Address) -> PositionView {
        let st = state(&env);
        let pos = position(&env, &depositor);
        let value = if st.total_shares == 0 {
            0
        } else {
            pos.shares * nav(&env, &st) / st.total_shares
        };
        PositionView {
            shares: pos.shares,
            value,
            principal: pos.principal,
            shortfall: (pos.principal - value).max(0),
        }
    }

    pub fn strategies(env: Env) -> Vec<Address> {
        strategies(&env)
    }

    pub fn config(env: Env) -> VaultConfig {
        config(&env)
    }

    pub fn state(env: Env) -> VaultState {
        state(&env)
    }

    // ----- operations ------------------------------------------------------

    /// Permissionless TTL maintenance for depositor positions.
    pub fn bump_position(env: Env, depositor: Address) {
        let key = DataKey::Position(depositor);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
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

fn config(env: &Env) -> VaultConfig {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn state(env: &Env) -> VaultState {
    env.storage()
        .instance()
        .get(&DataKey::State)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn save_state(env: &Env, st: &VaultState) {
    env.storage().instance().set(&DataKey::State, st);
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn strategies(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::Strategies)
        .unwrap_or_else(|| Vec::new(env))
}

fn nav(env: &Env, st: &VaultState) -> i128 {
    let mut total = st.idle;
    for strategy in strategies(env).iter() {
        total += StrategyClient::new(env, &strategy).total_value();
    }
    total
}

fn position(env: &Env, who: &Address) -> Position {
    env.storage()
        .persistent()
        .get(&DataKey::Position(who.clone()))
        .unwrap_or_default()
}

fn save_position(env: &Env, who: &Address, pos: &Position) {
    let key = DataKey::Position(who.clone());
    env.storage().persistent().set(&key, pos);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[cfg(test)]
mod test;
