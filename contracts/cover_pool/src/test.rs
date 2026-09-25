#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

const LOCK: u32 = 100;
/// 2.5% of the limit, the rate used in the demo.
const PREMIUM_BPS: u32 = 250;
/// 1 XLM in stroops.
const XLM: i128 = 10_000_000;

struct Setup<'a> {
    env: Env,
    pool: CoverPoolClient<'a>,
    token: TokenClient<'a>,
    admin: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let asset = sac.address();

    let pool_id = env.register(CoverPool, (admin.clone(), asset.clone(), LOCK, PREMIUM_BPS));
    let pool = CoverPoolClient::new(&env, &pool_id);

    Setup {
        token: TokenClient::new(&env, &asset),
        env,
        pool,
        admin,
    }
}

fn funded(s: &Setup, amount: i128) -> Address {
    let who = Address::generate(&s.env);
    StellarAssetClient::new(&s.env, &s.token.address).mint(&who, &amount);
    who
}

/// Whitelists a fresh vault address and opens `limit` of cover on it, paid
/// by a freshly funded curator. Returns the vault.
fn open(s: &Setup, limit: i128) -> Address {
    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);
    let curator = funded(s, limit); // more than enough for any premium
    s.pool.open_cover(&vault, &curator, &limit);
    vault
}

fn advance(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

// ----- staking -------------------------------------------------------------

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
    open(&s, 600);

    advance(&s.env, LOCK);
    s.pool.unstake(&alice, &500); // only 400 is free
}

#[test]
fn staker_joining_after_a_loss_is_not_charged_for_it() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let carol = funded(&s, 500);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 500);
    s.pool.claim(&vault, &500);

    // Share value is now 0.5, so 500 buys 1,000 shares worth exactly 500.
    assert_eq!(s.pool.stake(&carol, &500), 1_000);
    assert_eq!(s.pool.position(&carol).value, 500);
    assert_eq!(s.pool.position(&alice).value, 500);
}

// ----- cover ---------------------------------------------------------------

#[test]
fn open_cover_charges_premium_to_payer_and_credits_stakers() {
    let s = setup();
    let alice = funded(&s, 3_000);
    let bob = funded(&s, 1_000);
    s.pool.stake(&alice, &3_000);
    s.pool.stake(&bob, &1_000);

    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);
    let curator = funded(&s, 100);
    // 4,000 × 2.5% = 100
    assert_eq!(s.pool.open_cover(&vault, &curator, &4_000), 100);
    assert_eq!(s.token.balance(&curator), 0);

    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 4_000,
            claimed: 0
        }
    );
    assert_eq!(s.pool.available_capacity(), 0);
    // Premium is not capital: it cannot be slashed and is split by shares.
    assert_eq!(s.pool.state().total_capital, 4_000);
    assert_eq!(s.pool.position(&alice).premium_claimable, 75);
    assert_eq!(s.pool.position(&bob).premium_claimable, 25);
    assert_eq!(s.pool.claim_premium(&alice), 75);
    assert_eq!(s.token.balance(&alice), 75);
}

#[test]
fn open_cover_again_adds_to_remaining() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    let vault = open(&s, 300);
    let curator = funded(&s, 100);
    s.pool.open_cover(&vault, &curator, &200);

    assert_eq!(s.pool.cover(&vault).remaining, 500);
    assert_eq!(s.pool.state().coverage_written, 500);
}

#[test]
fn open_cover_cannot_exceed_capital() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    let vault = open(&s, 700);
    assert_eq!(s.pool.available_capacity(), 300);
    let curator = funded(&s, 100);
    assert!(s.pool.try_open_cover(&vault, &curator, &301).is_err());
}

#[test]
fn claim_slashes_every_staker_pro_rata_and_pays_the_vault() {
    let s = setup();
    let alice = funded(&s, 3_000);
    let bob = funded(&s, 1_000);
    s.pool.stake(&alice, &3_000);
    s.pool.stake(&bob, &1_000);

    let vault = open(&s, 1_000);
    assert_eq!(s.pool.claim(&vault, &1_000), 1_000);

    assert_eq!(s.token.balance(&vault), 1_000);
    assert_eq!(s.pool.state().total_capital, 3_000);
    assert_eq!(s.pool.state().coverage_written, 0);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 1_000
        }
    );
    // 25% loss, shared exactly in proportion; share counts unchanged.
    assert_eq!(s.pool.position(&alice).value, 2_250);
    assert_eq!(s.pool.position(&bob).value, 750);
    assert_eq!(s.pool.position(&alice).shares, 3_000);
    assert_eq!(s.pool.share_value(), 7_500_000);
}

#[test]
fn three_sequential_partial_claims_reduce_remaining_and_slash_stakers() {
    let s = setup();
    let alice = funded(&s, 600);
    let bob = funded(&s, 400);
    s.pool.stake(&alice, &600);
    s.pool.stake(&bob, &400);
    let vault = open(&s, 300);

    for i in 1..=3 {
        assert_eq!(s.pool.claim(&vault, &100), 100);
        let paid = 100 * i;
        assert_eq!(s.token.balance(&vault), paid, "claim {i}");
        assert_eq!(
            s.pool.cover(&vault),
            Cover {
                remaining: 300 - paid,
                claimed: paid,
            }
        );
        assert_eq!(s.pool.state().total_capital, 1_000 - paid);
        assert_eq!(s.pool.state().coverage_written, 300 - paid);
        // 60/40 split of every slash.
        assert_eq!(s.pool.position(&alice).value, 600 - paid * 6 / 10);
        assert_eq!(s.pool.position(&bob).value, 400 - paid * 4 / 10);
    }

    // The cover is exhausted; nothing more can be claimed.
    assert!(s.pool.try_claim(&vault, &1).is_err());
    assert_eq!(s.pool.state().coverage_written, 0);
    // The capital that backed the cover is free again.
    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &600), 420);
    assert_eq!(s.pool.unstake(&bob, &400), 280);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn claim_cannot_exceed_remaining_cover() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 400);
    s.pool.claim(&vault, &401);
}

#[test]
fn close_cover_frees_capacity_and_keeps_the_claimed_total() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 1_000);
    s.pool.claim(&vault, &100);
    s.pool.close_cover(&vault);

    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 100
        }
    );
    assert_eq!(s.pool.state().coverage_written, 0);
    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 900);
}

#[test]
fn demo_numbers_in_stroops() {
    // Section 1 of docs/PLAN.md: Alice stakes 1,000 XLM, the curator buys
    // 50 XLM of cover for 1.25 XLM, the vault claims a 10 XLM shortfall.
    let s = setup();
    let alice = funded(&s, 1_000 * XLM);
    s.pool.stake(&alice, &(1_000 * XLM));

    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);
    let curator = funded(&s, 2 * XLM);
    assert_eq!(s.pool.open_cover(&vault, &curator, &(50 * XLM)), 12_500_000);

    s.pool.claim(&vault, &(10 * XLM));
    assert_eq!(s.pool.position(&alice).value, 990 * XLM);
    assert_eq!(s.pool.position(&alice).premium_claimable, 12_500_000);
    assert_eq!(s.pool.cover(&vault).remaining, 40 * XLM);
    assert_eq!(s.token.balance(&vault), 10 * XLM);
}

// ----- access --------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn only_whitelisted_vaults_can_open_cover() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let stranger = Address::generate(&s.env);
    let curator = funded(&s, 100);
    s.pool.open_cover(&stranger, &curator, &100);
}

#[test]
fn claim_requires_the_vault_to_sign() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 500);

    // Drop blanket auth mocking: nobody has signed as the vault.
    s.env.set_auths(&[]);
    assert!(s.pool.try_claim(&vault, &100).is_err());
}

#[test]
fn only_the_admin_can_add_vaults() {
    let s = setup();
    s.env.set_auths(&[]);
    assert!(s.pool.try_add_vault(&Address::generate(&s.env)).is_err());
    let _ = &s.admin;
}
