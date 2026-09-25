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

/// Opens `limit` more cover on an already whitelisted vault, paid by a
/// freshly funded curator. Returns the premium charged.
fn open_more(s: &Setup, vault: &Address, limit: i128) -> i128 {
    let curator = funded(s, limit);
    s.pool.open_cover(vault, &curator, &limit)
}

fn advance(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

/// Asserts that a `try_*` call failed with exactly `want`.
#[track_caller]
fn expect_err<T: core::fmt::Debug, E: core::fmt::Debug>(
    r: Result<T, Result<soroban_sdk::Error, E>>,
    want: Error,
) {
    match r {
        Err(Ok(got)) => assert_eq!(got, soroban_sdk::Error::from(want)),
        other => panic!("expected {want:?}, got {other:?}"),
    }
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

// ----- claims and cover (ported from the Ethereum suites) -------------------

#[test]
fn claim_exhausts_cover_exactly_and_drained_pool_rejects_new_stake() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 1_000); // premium 25

    // One over the cover: rejected, nothing moves.
    expect_err(s.pool.try_claim(&vault, &1_001), Error::ClaimExceedsCover);
    assert_eq!(s.pool.state().total_capital, 1_000);
    assert_eq!(s.pool.state().coverage_written, 1_000);
    assert_eq!(s.token.balance(&vault), 0);

    // Exactly the cover: the pool is emptied.
    assert_eq!(s.pool.claim(&vault, &1_000), 1_000);
    assert_eq!(s.pool.state().total_capital, 0);
    assert_eq!(s.pool.state().coverage_written, 0);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 1_000
        }
    );
    assert_eq!(s.token.balance(&vault), 1_000);
    let a = s.pool.position(&alice);
    assert_eq!(a.shares, 1_000);
    assert_eq!(a.value, 0);
    assert_eq!(a.premium_claimable, 25); // premium survives the slash

    expect_err(s.pool.try_claim(&vault, &1), Error::NoCover);

    // Shares exist but capital is zero: no share price to mint at.
    let bob = funded(&s, 100);
    expect_err(s.pool.try_stake(&bob, &100), Error::PoolDrained);
    assert_eq!(s.token.balance(&bob), 100);
}

#[test]
fn partial_claims_decrement_remaining_and_accumulate_claimed() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 1_000);

    s.pool.claim(&vault, &300);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 700,
            claimed: 300
        }
    );
    s.pool.claim(&vault, &400);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 300,
            claimed: 700
        }
    );
    s.pool.claim(&vault, &300);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 1_000
        }
    );
    assert_eq!(s.pool.state().total_capital, 0);
    assert_eq!(s.token.balance(&vault), 1_000);
}

#[test]
fn two_vaults_share_capacity_and_one_vaults_claim_slashes_everyone() {
    let s = setup();
    let alice = funded(&s, 30_000);
    let bob = funded(&s, 10_000);
    s.pool.stake(&alice, &30_000);
    s.pool.stake(&bob, &10_000);

    let v1 = Address::generate(&s.env);
    let v2 = Address::generate(&s.env);
    s.pool.add_vault(&v1);
    s.pool.add_vault(&v2);
    assert_eq!(open_more(&s, &v1, 20_000), 500); // 2.5%
    assert_eq!(open_more(&s, &v2, 10_000), 250);
    // 750 total, split 75/25 by shares, each share rounded down.
    assert_eq!(s.pool.position(&alice).premium_claimable, 562); // 562.5
    assert_eq!(s.pool.position(&bob).premium_claimable, 187); // 187.5
    assert_eq!(s.pool.state().coverage_written, 30_000);
    assert_eq!(s.pool.available_capacity(), 10_000);

    // V1's claim slashes both stakers; V2's cover is untouched.
    s.pool.claim(&v1, &8_000);
    assert_eq!(s.pool.state().total_capital, 32_000);
    assert_eq!(s.pool.state().coverage_written, 22_000);
    assert_eq!(
        s.pool.cover(&v1),
        Cover {
            remaining: 12_000,
            claimed: 8_000
        }
    );
    assert_eq!(
        s.pool.cover(&v2),
        Cover {
            remaining: 10_000,
            claimed: 0
        }
    );
    assert_eq!(s.pool.position(&alice).value, 24_000);
    assert_eq!(s.pool.position(&bob).value, 8_000);
    // Claims reduce capital and coverage equally: free capacity is unchanged.
    assert_eq!(s.pool.available_capacity(), 10_000);

    s.pool.claim(&v2, &10_000);
    assert_eq!(s.pool.state().total_capital, 22_000);
    assert_eq!(s.pool.state().coverage_written, 12_000);
    assert_eq!(
        s.pool.cover(&v2),
        Cover {
            remaining: 0,
            claimed: 10_000
        }
    );
    assert_eq!(s.pool.position(&alice).value, 16_500);
    assert_eq!(s.pool.position(&bob).value, 5_500);

    // V2 is exhausted even though V1 still has cover left.
    expect_err(s.pool.try_claim(&v2, &1), Error::NoCover);

    // V2 can re-open up to the free capacity, not one unit more.
    let curator = funded(&s, 1_000);
    expect_err(
        s.pool.try_open_cover(&v2, &curator, &10_001),
        Error::InsufficientCapacity,
    );
    s.pool.open_cover(&v2, &curator, &10_000);
    assert_eq!(
        s.pool.cover(&v2),
        Cover {
            remaining: 10_000,
            claimed: 10_000
        }
    );
    assert_eq!(s.pool.available_capacity(), 0);
}

#[test]
fn staker_leaving_between_claims_takes_only_the_loss_so_far() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let bob = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.stake(&bob, &1_000);
    let vault = open(&s, 1_000); // premium 25 -> 12 each, rounded down

    s.pool.claim(&vault, &200);
    assert_eq!(s.pool.position(&alice).value, 900);
    assert_eq!(s.pool.position(&bob).value, 900);

    // 1,800 - 900 = 900 >= 800 still covered, so Alice may leave.
    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 900);
    assert_eq!(s.token.balance(&alice), 900);
    assert_eq!(s.pool.state().total_capital, 900);
    assert_eq!(s.pool.state().total_shares, 1_000);

    // The rest of the loss lands on Bob alone.
    s.pool.claim(&vault, &800);
    assert_eq!(s.pool.state().total_capital, 100);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 1_000
        }
    );
    assert_eq!(s.pool.position(&bob).value, 100);
    assert_eq!(s.pool.position(&alice).value, 0);
    // Premium was split equally and is untouched by the claims.
    assert_eq!(s.pool.position(&alice).premium_claimable, 12);
    assert_eq!(s.pool.position(&bob).premium_claimable, 12);
}

#[test]
fn capacity_is_exact_and_only_closing_cover_frees_it() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    let v1 = open(&s, 600);
    let v2 = open(&s, 400);
    assert_eq!(s.pool.available_capacity(), 0);

    let curator = funded(&s, 100);
    expect_err(
        s.pool.try_open_cover(&v2, &curator, &1),
        Error::InsufficientCapacity,
    );
    let stranger = Address::generate(&s.env);
    expect_err(
        s.pool.try_open_cover(&stranger, &curator, &1),
        Error::NotWhitelisted,
    );

    // A claim never frees capacity: capital and coverage fall together.
    s.pool.claim(&v1, &100);
    assert_eq!(s.pool.state().total_capital, 900);
    assert_eq!(s.pool.state().coverage_written, 900);
    assert_eq!(s.pool.available_capacity(), 0);

    s.pool.close_cover(&v2);
    assert_eq!(s.pool.available_capacity(), 400);
}

#[test]
fn open_twice_partial_claim_then_close_releases_the_rest() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);

    let vault = open(&s, 300); // premium 7 (7.5 rounded down)
    assert_eq!(open_more(&s, &vault, 200), 5);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 500,
            claimed: 0
        }
    );
    assert_eq!(s.pool.state().coverage_written, 500);

    s.pool.claim(&vault, &400);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 100,
            claimed: 400
        }
    );
    assert_eq!(s.pool.state().total_capital, 600);
    assert_eq!(s.pool.state().coverage_written, 100);

    s.pool.close_cover(&vault);
    assert_eq!(
        s.pool.cover(&vault),
        Cover {
            remaining: 0,
            claimed: 400
        }
    );
    assert_eq!(s.pool.state().coverage_written, 0);
    expect_err(s.pool.try_claim(&vault, &1), Error::NoCover);

    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 600);
    assert_eq!(s.pool.position(&alice).premium_claimable, 12);
    assert_eq!(s.pool.claim_premium(&alice), 12);
    assert_eq!(s.token.balance(&alice), 612);
}

#[test]
fn open_cover_fails_atomically_when_payer_cannot_pay_the_premium() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);

    let broke = Address::generate(&s.env); // balance 0
    assert!(s.pool.try_open_cover(&vault, &broke, &400).is_err());

    assert_eq!(s.pool.cover(&vault), Cover::default());
    assert_eq!(s.pool.state().coverage_written, 0);
    assert_eq!(s.pool.state().premium_index, 0);
    assert_eq!(s.pool.state().premiums_paid, 0);
    assert_eq!(s.pool.position(&alice).premium_claimable, 0);
}

// ----- premium index ---------------------------------------------------------

#[test]
fn premium_splits_by_shares_and_is_not_capital() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let bob = funded(&s, 3_000);
    s.pool.stake(&alice, &1_000);
    s.pool.stake(&bob, &3_000);

    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);
    assert_eq!(open_more(&s, &vault, 4_000), 100);

    assert_eq!(s.pool.position(&alice).premium_claimable, 25);
    assert_eq!(s.pool.position(&bob).premium_claimable, 75);
    assert_eq!(s.pool.state().total_capital, 4_000);
    assert_eq!(s.pool.state().premiums_paid, 100);
    assert_eq!(s.token.balance(&s.pool.address), 4_100);
    assert_eq!(s.pool.available_capacity(), 0);

    // Every unit of capital backs cover: not even one share can leave.
    advance(&s.env, LOCK);
    expect_err(
        s.pool.try_unstake(&alice, &1),
        Error::CapitalBackingCoverage,
    );
}

#[test]
fn late_staker_earns_no_premium_from_before_they_joined() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 400); // p1 = 10, all to Alice
    assert_eq!(s.pool.position(&alice).premium_claimable, 10);

    let bob = funded(&s, 1_000);
    s.pool.stake(&bob, &1_000);
    assert_eq!(s.pool.position(&bob).premium_claimable, 0);

    assert_eq!(open_more(&s, &vault, 800), 20); // p2 = 20, half each
    assert_eq!(s.pool.position(&alice).premium_claimable, 20);
    assert_eq!(s.pool.position(&bob).premium_claimable, 10);
    assert_eq!(s.pool.state().premiums_paid, 30);
}

#[test]
fn unstaker_keeps_earned_premium_and_earns_nothing_after() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let bob = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    s.pool.stake(&bob, &1_000);
    let vault = open(&s, 800); // premium 20, 10 each

    // 2,000 - 1,000 >= 800: Alice may leave entirely.
    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &1_000), 1_000);
    let a = s.pool.position(&alice);
    assert_eq!(a.shares, 0);
    assert_eq!(a.premium_claimable, 10);

    // New premium goes only to the remaining staker.
    s.pool.close_cover(&vault);
    assert_eq!(open_more(&s, &vault, 800), 20);
    assert_eq!(s.pool.position(&bob).premium_claimable, 30);
    assert_eq!(s.pool.position(&alice).premium_claimable, 10);

    assert_eq!(s.pool.claim_premium(&alice), 10);
    assert_eq!(s.token.balance(&alice), 1_010);
    assert_eq!(s.pool.claim_premium(&alice), 0);
    assert_eq!(s.token.balance(&alice), 1_010);
}

#[test]
fn premium_dust_stays_in_the_pool_and_the_index_never_falls() {
    let s = setup();
    let alice = funded(&s, 100);
    let bob = funded(&s, 200);
    s.pool.stake(&alice, &100);
    s.pool.stake(&bob, &200);
    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);

    let mut last_index = 0;
    for _ in 0..3 {
        assert_eq!(open_more(&s, &vault, 100), 2); // 2.5 rounded down
        let index = s.pool.state().premium_index;
        assert!(index > last_index);
        last_index = index;

        let owed =
            s.pool.position(&alice).premium_claimable + s.pool.position(&bob).premium_claimable;
        let received = s.pool.state().premiums_paid;
        assert!(owed <= received, "owed {owed} > received {received}");
        // Dust never comes out of capital.
        assert!(s.token.balance(&s.pool.address) >= s.pool.state().total_capital + owed);
    }
    assert_eq!(s.pool.state().premiums_paid, 6);
    assert_eq!(s.pool.state().coverage_written, 300);
    // 6 × (1/3) and 6 × (2/3) with every checkpoint rounded down.
    assert_eq!(s.pool.position(&alice).premium_claimable, 1);
    assert_eq!(s.pool.position(&bob).premium_claimable, 3);
    assert_eq!(s.pool.claim_premium(&alice) + s.pool.claim_premium(&bob), 4);
    assert_eq!(s.token.balance(&s.pool.address), 302);
}

#[test]
fn premium_rounds_down() {
    let s = setup();
    let alice = funded(&s, 2_000);
    s.pool.stake(&alice, &2_000);
    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);

    let curator = funded(&s, 100);
    // 1,010 × 2.5% = 25.25
    assert_eq!(s.pool.open_cover(&vault, &curator, &1_010), 25);
    assert_eq!(s.token.balance(&curator), 75);
    assert_eq!(s.pool.state().premiums_paid, 25);
    assert_eq!(s.pool.position(&alice).premium_claimable, 25);
}

// ----- staking, lock and rounding -------------------------------------------

#[test]
fn unstake_up_to_the_capital_backing_limit_then_all_after_close() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 600);
    advance(&s.env, LOCK);

    expect_err(
        s.pool.try_unstake(&alice, &401),
        Error::CapitalBackingCoverage,
    );
    assert_eq!(s.pool.unstake(&alice, &400), 400);
    assert_eq!(s.pool.state().total_capital, 600);
    assert_eq!(s.pool.state().total_shares, 600);
    expect_err(
        s.pool.try_unstake(&alice, &1),
        Error::CapitalBackingCoverage,
    );

    s.pool.close_cover(&vault);
    assert_eq!(s.pool.unstake(&alice, &600), 600);
    assert_eq!(s.pool.state().total_capital, 0);
    assert_eq!(s.pool.state().total_shares, 0);
    assert_eq!(s.token.balance(&alice), 1_000);

    // The pool is empty again, so shares mint one to one.
    let carol = funded(&s, 50);
    assert_eq!(s.pool.stake(&carol, &50), 50);
}

#[test]
fn lock_boundary_is_inclusive_and_restaking_relocks_the_whole_position() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let start = s.env.ledger().sequence();
    s.pool.stake(&alice, &500);
    assert_eq!(s.pool.position(&alice).locked_until, start + LOCK);

    advance(&s.env, LOCK - 1);
    expect_err(s.pool.try_unstake(&alice, &100), Error::StillLocked);
    advance(&s.env, 1);
    assert_eq!(s.pool.unstake(&alice, &100), 100);

    // Staking again resets the lock for every share, old ones included.
    advance(&s.env, 10);
    let now = s.env.ledger().sequence();
    s.pool.stake(&alice, &500);
    assert_eq!(s.pool.position(&alice).locked_until, now + LOCK);
    assert_eq!(s.pool.position(&alice).shares, 900);
    expect_err(s.pool.try_unstake(&alice, &1), Error::StillLocked);
    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&alice, &900), 900);
}

#[test]
fn share_price_after_an_odd_slash_rounds_in_the_pools_favour() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 1_000);

    s.pool.claim(&vault, &1);
    assert_eq!(s.pool.state().total_capital, 999);
    assert_eq!(s.pool.state().total_shares, 1_000);

    // 1,000 × 1,000 / 999 = 1,001.001 -> 1,001 shares.
    let bob = funded(&s, 1_000);
    assert_eq!(s.pool.stake(&bob, &1_000), 1_001);
    assert_eq!(s.pool.state().total_capital, 1_999);
    assert_eq!(s.pool.state().total_shares, 2_001);
    assert_eq!(s.pool.position(&alice).value, 999);
    assert_eq!(s.pool.position(&bob).value, 999);
    // The unit of dust stays with the pool.
    assert_eq!(
        s.pool.state().total_capital - s.pool.position(&alice).value - s.pool.position(&bob).value,
        1
    );

    advance(&s.env, LOCK);
    assert_eq!(s.pool.unstake(&bob, &1_001), 999);
    assert_eq!(s.pool.position(&alice).value, 1_000);
    assert_eq!(s.pool.state().total_capital, 1_000);
}

#[test]
fn one_unit_stake_is_wiped_by_a_one_unit_claim() {
    let s = setup();
    let alice = funded(&s, 1);
    let bob = funded(&s, 1_000_000);
    s.pool.stake(&alice, &1);
    s.pool.stake(&bob, &1_000_000);

    let vault = Address::generate(&s.env);
    s.pool.add_vault(&vault);
    let curator = funded(&s, 10);
    // 1 × 2.5% rounds to zero: no premium is charged or credited.
    assert_eq!(s.pool.open_cover(&vault, &curator, &1), 0);
    assert_eq!(s.token.balance(&curator), 10);
    assert_eq!(s.pool.state().premiums_paid, 0);
    assert_eq!(s.pool.state().premium_index, 0);

    s.pool.claim(&vault, &1);
    assert_eq!(s.pool.position(&alice).value, 0);
    assert_eq!(s.pool.position(&bob).value, 999_999);
}

#[test]
fn unstake_worth_zero_is_rejected() {
    let s = setup();
    let alice = funded(&s, 1_000);
    s.pool.stake(&alice, &1_000);
    let vault = open(&s, 1_000);
    s.pool.claim(&vault, &999);
    assert_eq!(s.pool.state().total_capital, 1);

    // Share price is 0.001: one unit buys 1,000 shares.
    let bob = funded(&s, 1);
    assert_eq!(s.pool.stake(&bob, &1), 1_000);
    assert_eq!(s.pool.state().total_capital, 2);
    assert_eq!(s.pool.state().total_shares, 2_000);

    advance(&s.env, LOCK);
    // 999 × 2 / 2,000 rounds to zero: nothing to pay, so nothing to burn.
    expect_err(s.pool.try_unstake(&alice, &999), Error::InvalidAmount);
    assert_eq!(s.pool.position(&alice).shares, 1_000);
    // 1,000 × 2 / 2,000 = 1, and 2 - 1 still backs the 1 of cover left.
    assert_eq!(s.pool.unstake(&bob, &1_000), 1);
    assert_eq!(s.pool.state().total_capital, 1);
    assert_eq!(s.pool.state().coverage_written, 1);
}

// ----- invariants ------------------------------------------------------------

/// Checks the pool's accounting identities against every staker and vault.
#[track_caller]
fn check_invariants(s: &Setup, stakers: &[&Address], vaults: &[&Address], step: &str) {
    let st = s.pool.state();

    let mut shares = 0;
    let mut values = 0;
    let mut premium_owed = 0;
    for who in stakers {
        let p = s.pool.position(who);
        shares += p.shares;
        values += p.value;
        premium_owed += p.premium_claimable;
    }
    assert_eq!(shares, st.total_shares, "{step}: shares");
    assert!(values <= st.total_capital, "{step}: values");
    assert!(
        st.total_capital - values < stakers.len() as i128,
        "{step}: dust {} with {} stakers",
        st.total_capital - values,
        stakers.len()
    );

    let mut remaining = 0;
    let mut claimed = 0;
    let mut paid_to_vaults = 0;
    for v in vaults {
        let c = s.pool.cover(v);
        remaining += c.remaining;
        claimed += c.claimed;
        paid_to_vaults += s.token.balance(v);
    }
    assert_eq!(remaining, st.coverage_written, "{step}: coverage");
    assert!(
        st.coverage_written <= st.total_capital,
        "{step}: collateral"
    );
    assert_eq!(claimed, st.claims_paid, "{step}: claims");
    assert_eq!(claimed, paid_to_vaults, "{step}: vault balances");

    assert!(
        s.token.balance(&s.pool.address) >= st.total_capital + premium_owed,
        "{step}: pool balance"
    );
}

#[test]
fn accounting_invariants_hold_across_a_scripted_sequence() {
    let s = setup();
    let alice = funded(&s, 1_000);
    let bob = funded(&s, 3_000);
    let carol = funded(&s, 500);
    let v1 = Address::generate(&s.env);
    let v2 = Address::generate(&s.env);
    s.pool.add_vault(&v1);
    s.pool.add_vault(&v2);
    let stakers = [&alice, &bob, &carol];
    let vaults = [&v1, &v2];
    // Positions for stakers who have not staked yet do not exist, so the
    // checker only sees a staker once they are in.
    let check = |who: &[&Address], step: &str| check_invariants(&s, who, &vaults, step);

    s.pool.stake(&alice, &1_000);
    check(&stakers[..1], "alice stakes");
    s.pool.stake(&bob, &3_000);
    check(&stakers[..2], "bob stakes");

    open_more(&s, &v1, 2_000);
    check(&stakers[..2], "v1 opens");
    open_more(&s, &v2, 1_000);
    check(&stakers[..2], "v2 opens");

    s.pool.claim(&v1, &500);
    check(&stakers[..2], "v1 claims 500");

    s.pool.stake(&carol, &500);
    check(&stakers, "carol stakes after a loss");

    s.pool.claim(&v2, &300);
    check(&stakers, "v2 claims 300");
    s.pool.claim(&v1, &700);
    check(&stakers, "v1 claims 700");

    advance(&s.env, LOCK);
    s.pool.unstake(&bob, &1_000);
    check(&stakers, "bob unstakes 1,000 shares");

    s.pool.close_cover(&v1);
    check(&stakers, "v1 closes");

    s.pool.claim_premium(&alice);
    check(&stakers, "alice claims premium");

    open_more(&s, &v1, 500);
    check(&stakers, "v1 re-opens");
    s.pool.claim(&v1, &500);
    check(&stakers, "v1 exhausts its new cover");

    // A few spot values for the record.
    let st = s.pool.state();
    assert_eq!(st.claims_paid, 2_000);
    assert_eq!(st.coverage_written, 700);
    assert_eq!(s.pool.cover(&v1).claimed, 1_700);
    assert_eq!(s.pool.cover(&v2).claimed, 300);
}
