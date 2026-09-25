//! Policy — parametric bad-debt coverage for a position in a Blend pool.
//!
//! A vault operator buys cover on a supply position in a Blend reserve. At
//! issuance the policy records the reserve's `b_rate` as a baseline and
//! reserves its limit in the CoverPool. If bad debt is later socialized across
//! suppliers, Blend lowers `b_rate`; once the decline passes the trigger
//! threshold, anyone may call `settle` and the covered loss is paid to the
//! beneficiary automatically. No claims process, no vote.

#![no_std]

use catalysis_common::{AdapterClient, CoverPoolClient, BPS};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error, Address,
    Env,
};

const TTL_THRESHOLD: u32 = 17_280 * 15;
const TTL_EXTEND_TO: u32 = 17_280 * 30;

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// Underwriting terms applied to every policy this contract issues.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Terms {
    pub admin: Address,
    pub pool: Address,
    pub adapter: Address,
    /// Premium charged at issuance, as bps of the limit.
    pub premium_bps: u32,
    /// Minimum decline in the covered rate that makes a policy payable.
    pub trigger_bps: u32,
    /// Policy lifetime in ledgers.
    pub duration_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Active,
    Paid,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub buyer: Address,
    pub beneficiary: Address,
    /// The Blend pool holding the covered position.
    pub target: Address,
    /// The reserve asset supplied in that pool.
    pub asset: Address,
    /// Size of the covered supply position, in the reserve asset's units.
    pub covered_amount: i128,
    /// Maximum payout, reserved in the CoverPool.
    pub limit: i128,
    pub premium: i128,
    /// Covered rate (`b_rate`) observed at issuance.
    pub baseline_rate: i128,
    pub start_ledger: u32,
    pub end_ledger: u32,
    pub status: Status,
    pub payout: i128,
}

/// Live reading of a policy against its trigger.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Check {
    pub baseline_rate: i128,
    pub current_rate: i128,
    pub decline_bps: i128,
    pub payable: bool,
    pub payout: i128,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Terms,
    NextId,
    Policy(u64),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    UnknownPolicy = 2,
    NotActive = 3,
    InvalidAmount = 4,
    NotTriggered = 5,
    Expired = 6,
    NotYetExpired = 7,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[contractevent]
pub struct PolicyIssued {
    #[topic]
    pub policy_id: u64,
    #[topic]
    pub buyer: Address,
    pub limit: i128,
    pub premium: i128,
    pub baseline_rate: i128,
    pub end_ledger: u32,
}

#[contractevent]
pub struct PolicyPaid {
    #[topic]
    pub policy_id: u64,
    #[topic]
    pub beneficiary: Address,
    pub payout: i128,
    pub decline_bps: i128,
}

#[contractevent]
pub struct PolicyExpired {
    #[topic]
    pub policy_id: u64,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PolicyContract;

#[contractimpl]
impl PolicyContract {
    pub fn __constructor(
        env: Env,
        admin: Address,
        pool: Address,
        adapter: Address,
        premium_bps: u32,
        trigger_bps: u32,
        duration_ledgers: u32,
    ) {
        env.storage().instance().set(
            &DataKey::Terms,
            &Terms {
                admin,
                pool,
                adapter,
                premium_bps,
                trigger_bps,
                duration_ledgers,
            },
        );
        env.storage().instance().set(&DataKey::NextId, &1u64);
    }

    /// Buys cover on `covered_amount` supplied to `asset` in Blend pool
    /// `target`, paying out up to `limit` to `beneficiary`. The premium is
    /// charged to `buyer` at issuance and goes to CoverPool stakers.
    pub fn issue(
        env: Env,
        buyer: Address,
        beneficiary: Address,
        target: Address,
        asset: Address,
        covered_amount: i128,
        limit: i128,
    ) -> u64 {
        buyer.require_auth();
        if covered_amount <= 0 || limit <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let t = terms(&env);
        let id: u64 = env.storage().instance().get(&DataKey::NextId).unwrap();

        let baseline_rate = AdapterClient::new(&env, &t.adapter).observe(&target, &asset);
        let premium = limit * t.premium_bps as i128 / BPS;
        let start_ledger = env.ledger().sequence();
        let end_ledger = start_ledger + t.duration_ledgers;

        let pool = CoverPoolClient::new(&env, &t.pool);
        pool.reserve(&id, &limit, &beneficiary);
        pool.accrue_premium(&id, &buyer, &premium);

        let policy = Policy {
            buyer: buyer.clone(),
            beneficiary,
            target,
            asset,
            covered_amount,
            limit,
            premium,
            baseline_rate,
            start_ledger,
            end_ledger,
            status: Status::Active,
            payout: 0,
        };
        save(&env, id, &policy);
        env.storage().instance().set(&DataKey::NextId, &(id + 1));
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);

        PolicyIssued {
            policy_id: id,
            buyer,
            limit,
            premium,
            baseline_rate,
            end_ledger,
        }
        .publish(&env);
        id
    }

    /// Reads the covered rate now and reports whether the policy is payable.
    pub fn check(env: Env, policy_id: u64) -> Check {
        let p = load(&env, policy_id);
        evaluate(&env, &terms(&env), &p)
    }

    /// Permissionless. Pays the covered loss to the beneficiary recorded at
    /// issuance once the trigger condition holds.
    pub fn settle(env: Env, policy_id: u64) -> i128 {
        let t = terms(&env);
        let mut p = load(&env, policy_id);
        if p.status != Status::Active {
            panic_with_error!(&env, Error::NotActive);
        }
        if env.ledger().sequence() > p.end_ledger {
            panic_with_error!(&env, Error::Expired);
        }
        let c = evaluate(&env, &t, &p);
        if !c.payable {
            panic_with_error!(&env, Error::NotTriggered);
        }

        p.status = Status::Paid;
        p.payout = c.payout;
        save(&env, policy_id, &p);

        CoverPoolClient::new(&env, &t.pool).settle_claim(&policy_id, &c.payout);
        PolicyPaid {
            policy_id,
            beneficiary: p.beneficiary,
            payout: c.payout,
            decline_bps: c.decline_bps,
        }
        .publish(&env);
        c.payout
    }

    /// Permissionless. Closes a policy past its end ledger and returns its
    /// reserved capacity to the pool.
    pub fn expire(env: Env, policy_id: u64) {
        let t = terms(&env);
        let mut p = load(&env, policy_id);
        if p.status != Status::Active {
            panic_with_error!(&env, Error::NotActive);
        }
        if env.ledger().sequence() <= p.end_ledger {
            panic_with_error!(&env, Error::NotYetExpired);
        }
        p.status = Status::Expired;
        save(&env, policy_id, &p);
        CoverPoolClient::new(&env, &t.pool).release(&policy_id);
        PolicyExpired { policy_id }.publish(&env);
    }

    pub fn get(env: Env, policy_id: u64) -> Policy {
        load(&env, policy_id)
    }

    pub fn terms(env: Env) -> Terms {
        terms(&env)
    }

    /// Replaces the contract Wasm. Admin only.
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
        terms(&env).admin.require_auth();
        env.deployer()
            .update_current_contract(soroban_sdk::ContractExecutable::Wasm(new_wasm_hash));
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn evaluate(env: &Env, t: &Terms, p: &Policy) -> Check {
    let current_rate = AdapterClient::new(env, &t.adapter).observe(&p.target, &p.asset);
    let decline_bps = if current_rate >= p.baseline_rate {
        0
    } else {
        (p.baseline_rate - current_rate) * BPS / p.baseline_rate
    };
    let payable = decline_bps >= t.trigger_bps as i128;
    // Loss on the covered position is proportional to the rate decline.
    let loss = p.covered_amount * (p.baseline_rate - current_rate).max(0) / p.baseline_rate;
    let payout = if payable { loss.min(p.limit) } else { 0 };
    Check {
        baseline_rate: p.baseline_rate,
        current_rate,
        decline_bps,
        payable,
        payout,
    }
}

fn terms(env: &Env) -> Terms {
    env.storage()
        .instance()
        .get(&DataKey::Terms)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotInitialized))
}

fn load(env: &Env, id: u64) -> Policy {
    env.storage()
        .persistent()
        .get(&DataKey::Policy(id))
        .unwrap_or_else(|| panic_with_error!(env, Error::UnknownPolicy))
}

fn save(env: &Env, id: u64, p: &Policy) {
    let key = DataKey::Policy(id);
    env.storage().persistent().set(&key, p);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
}
