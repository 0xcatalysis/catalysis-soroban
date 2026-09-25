#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

const LOCK: u32 = 100;

struct Setup<'a> {
    env: Env,
    pool: CoverPoolClient<'a>,
    token: TokenClient<'a>,
    policy: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let asset = sac.address();

    let pool_id = env.register(CoverPool, (admin.clone(), asset.clone(), LOCK));
    let pool = CoverPoolClient::new(&env, &pool_id);
    let policy = Address::generate(&env);
    pool.set_policy_contract(&policy);

    Setup {
        token: TokenClient::new(&env, &asset),
        env,
        pool,
        policy,
    }
}

fn funded(s: &Setup, amount: i128) -> Address {
    let who = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token.address).mint(&who, &amount);
    who
}

fn advance(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

#[test]
fn stake_mints_shares_one_to_one_on_empty_pool() {
    let s = setup();
    let alice = funded(&s, 1_000);

    assert_eq!(s.pool.stake(&alice, &1_000), 1_000);
    assert_eq!(s.pool.state().total_capital, 1_000);
    assert_eq!(s.token.balance(&alice), 0);
    assert_eq!(s.token.balance(&s.pool.address), 1_000);
}

#[test]
fn unstake_after_lock_returns_capital() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 1_000);
    assert_eq!(s.token.balance(&alice), 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn unstake_before_lock_is_rejected() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.unstake(&alice, &1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn capital_backing_coverage_cannot_leave() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.reserve(&1, &600, &Address::generate(&s.env));

    advance(&s.env, LOCK);
    s.pool.unstake(&alice, &500); // only 400 is free
}

#[test]
fn reserve_cannot_exceed_capital() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    s.pool.reserve(&1, &700, &Address::generate(&s.env));
    assert_eq!(s.pool.available_capacity(), 300);
    assert!(s
        .pool
        .try_reserve(&2, &301, &Address::generate(&s.env))
        .is_err());
}

#[test]
fn premium_is_shared_pro_rata() {
    let s = setup();
    let alice = funded(&s, 3_000);
    let bob = funded(&s, 1_000);
    let buyer = funded(&s, 400);
    s.pool.stake(&alice, &3_000);
    s.pool.stake(&bob, &1_000);

    s.pool.reserve(&1, &1_000, &buyer);
    s.pool.accrue_premium(&1, &buyer, &400);

    assert_eq!(s.pool.position(&alice).premium_claimable, 300);
    assert_eq!(s.pool.position(&bob).premium_claimable, 100);
    assert_eq!(s.pool.claim_premium(&alice), 300);
    assert_eq!(s.token.balance(&alice), 300);
}

#[test]
fn claim_slashes_every_staker_pro_rata_and_pays_beneficiary() {
    let s = setup();
    let alice = funded(&s, 3_000);
    let bob = funded(&s, 1_000);
    let vault = Address::generate(&s.env);
    s.pool.stake(&alice, &3_000);
    s.pool.stake(&bob, &1_000);

    s.pool.reserve(&1, &1_000, &vault);
    s.pool.settle_claim(&1, &1_000);

    assert_eq!(s.token.balance(&vault), 1_000);
    assert_eq!(s.pool.state().total_capital, 3_000);
    assert_eq!(s.pool.state().coverage_written, 0);
    // 25% loss, shared exactly in proportion; share counts unchanged.
    assert_eq!(s.pool.position(&alice).value, 2_250);
    assert_eq!(s.pool.position(&bob).value, 750);
    assert_eq!(s.pool.position(&alice).shares, 3_000);
    assert_eq!(s.pool.share_value(), 7_500_000);
}

#[test]
fn staker_joining_after_a_loss_is_not_charged_for_it() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let carol = funded(&s, 500);
    s.pool.stake(&alice, &1_000);
    s.pool.reserve(&1, &500, &Address::generate(&s.env));
    s.pool.settle_claim(&1, &500);

    // Share value is now 0.5, so 500 buys 1,000 shares worth exactly 500.
    assert_eq!(s.pool.stake(&carol, &500), 1_000);
    assert_eq!(s.pool.position(&carol).value, 500);
    assert_eq!(s.pool.position(&alice).value, 500);
}

#[test]
fn release_frees_capacity() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.reserve(&1, &1_000, &Address::generate(&s.env));
    s.pool.release(&1);

    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 1_000);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn payout_cannot_exceed_reserved_limit() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.reserve(&1, &400, &Address::generate(&s.env));
    s.pool.settle_claim(&1, &401);
}

#[test]
fn only_the_policy_contract_can_reserve() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    // Drop blanket auth mocking: nobody has signed as the policy contract.
    s.env.set_auths(&[]);
    assert!(s
        .pool
        .try_reserve(&1, &100, &Address::generate(&s.env))
        .is_err());
    let _ = &s.policy;
}
