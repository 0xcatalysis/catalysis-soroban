#![cfg(test)]

use super::*;
use cover_pool::{CoverPool, CoverPoolClient as PoolClient};
use mock_strategy::{MockStrategy, MockStrategyClient};
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

/// 1 XLM in stroops.
const XLM: i128 = 10_000_000;
const LOCK: u32 = 100;
const PREMIUM_BPS: u32 = 250;

struct World<'a> {
    env: Env,
    token: TokenClient<'a>,
    pool: PoolClient<'a>,
    vault: CoveredVaultClient<'a>,
    market_a: MockStrategyClient<'a>,
    market_b: MockStrategyClient<'a>,
    /// Curator of the vault and admin of everything.
    admin: Address,
    /// The one staker in the pool.
    alice: Address,
}

/// Pool with Alice's 1,000 XLM staked, a vault whitelisted on it with two
/// empty strategies, and the curator holding 100 XLM for premiums.
fn world<'a>() -> World<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let asset = sac.address();
    let token = TokenClient::new(&env, &asset);

    let pool_id = env.register(CoverPool, (admin.clone(), asset.clone(), LOCK, PREMIUM_BPS));
    let pool = PoolClient::new(&env, &pool_id);

    let vault_id = env.register(
        CoveredVault,
        (admin.clone(), asset.clone(), pool_id.clone()),
    );
    let vault = CoveredVaultClient::new(&env, &vault_id);
    pool.add_vault(&vault_id);

    let mut markets = [None, None];
    for slot in markets.iter_mut() {
        let id = env.register(
            MockStrategy,
            (admin.clone(), vault_id.clone(), asset.clone()),
        );
        vault.add_strategy(&id);
        *slot = Some(MockStrategyClient::new(&env, &id));
    }
    let [market_a, market_b] = markets.map(|m| m.unwrap());

    let alice = Address::generate(&env);
    StellarAssetClient::new(&env, &asset).mint(&alice, &(1_000 * XLM));
    pool.stake(&alice, &(1_000 * XLM));
    StellarAssetClient::new(&env, &asset).mint(&admin, &(100 * XLM));

    World {
        env,
        token,
        pool,
        vault,
        market_a,
        market_b,
        admin,
        alice,
    }
}

fn funded(w: &World, amount: i128) -> Address {
    let who = Address::generate(&w.env);
    StellarAssetClient::new(&w.env, &w.token.address).mint(&who, &amount);
    who
}

/// Demo steps 0 to 3: cover bought, user deposits 100, 50 in each market.
fn demo_allocated<'a>(w: &World<'a>) -> Address {
    w.vault.buy_cover(&(50 * XLM));
    let user = funded(w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));
    w.vault.allocate(&w.market_a.address, &(50 * XLM));
    w.vault.allocate(&w.market_b.address, &(50 * XLM));
    user
}

/// A fresh depositor who has put `amount` into the vault.
fn deposit(w: &World, amount: i128) -> Address {
    let who = funded(w, amount);
    w.vault.deposit(&who, &amount);
    who
}

/// Curator moves everything idle into market A.
fn allocate_all(w: &World) {
    w.vault.allocate(&w.market_a.address, &w.vault.state().idle);
}

/// Stands in for yield: mints straight to the strategy, raising its value.
fn donate(w: &World, strategy: &MockStrategyClient, amount: i128) {
    StellarAssetClient::new(&w.env, &w.token.address).mint(&strategy.address, &amount);
}

/// Buys `limit` of cover, staking extra capital first when Alice's 1,000 XLM
/// does not cover it.
fn buy_cover(w: &World, limit: i128) {
    let capacity = w.pool.available_capacity();
    if limit > capacity {
        let staker = funded(w, limit - capacity);
        w.pool.stake(&staker, &(limit - capacity));
    }
    w.vault.buy_cover(&limit);
}

fn claimed(w: &World) -> i128 {
    w.pool.cover(&w.vault.address).claimed
}

fn remaining(w: &World) -> i128 {
    w.pool.cover(&w.vault.address).remaining
}

fn view(shares: i128, value: i128, principal: i128, shortfall: i128) -> PositionView {
    PositionView {
        shares,
        value,
        principal,
        shortfall,
    }
}

/// Sums every listed position and checks the books balance: shares add up to
/// `total_shares`, principals to `principal`, values never exceed the NAV, and
/// the NAV is idle plus both strategies.
fn assert_books(w: &World, users: &[&Address], principal: i128) {
    let st = w.vault.state();
    let (mut shares, mut principals, mut values) = (0, 0, 0);
    for u in users {
        let p = w.vault.position(u);
        shares += p.shares;
        principals += p.principal;
        values += p.value;
    }
    assert_eq!(shares, st.total_shares);
    assert_eq!(principals, principal);
    assert!(values <= w.vault.nav());
    assert_eq!(
        w.vault.nav(),
        st.idle + w.market_a.total_value() + w.market_b.total_value()
    );
}

/// Withdraws and checks that the depositor got `max(value, protected)`,
/// with `value` and `protected` computed from the state before the call.
/// Returns what was paid.
fn withdraw_checked(w: &World, who: &Address, shares: i128) -> i128 {
    let st = w.vault.state();
    let before = w.vault.position(who);
    let value = shares * w.vault.nav() / st.total_shares;
    let protected = before.principal * shares / before.shares;
    let claimed_before = claimed(w);
    let balance_before = w.token.balance(who);

    let paid = w.vault.withdraw(who, &shares);

    assert_eq!(paid, value.max(protected));
    assert_eq!(w.token.balance(who), balance_before + paid);
    assert_eq!(claimed(w) - claimed_before, (protected - value).max(0));
    let after = w.vault.position(who);
    assert_eq!(after.shares, before.shares - shares);
    assert_eq!(after.principal, before.principal - protected);
    paid
}

// ----- deposits and NAV ----------------------------------------------------

#[test]
fn first_deposit_mints_shares_one_to_one() {
    let w = world();
    let user = funded(&w, 100 * XLM);

    assert_eq!(w.vault.deposit(&user, &(100 * XLM)), 100 * XLM);

    assert_eq!(w.vault.state().total_shares, 100 * XLM);
    assert_eq!(w.vault.state().idle, 100 * XLM);
    assert_eq!(w.vault.nav(), 100 * XLM);
    assert_eq!(w.token.balance(&w.vault.address), 100 * XLM);
    let p = w.vault.position(&user);
    assert_eq!(p.shares, 100 * XLM);
    assert_eq!(p.principal, 100 * XLM);
    assert_eq!(p.value, 100 * XLM);
    assert_eq!(p.shortfall, 0);
}

#[test]
fn later_deposits_are_priced_at_nav() {
    let w = world();
    let user = demo_allocated(&w);
    // Market B loses 20%: NAV 100 -> 90 with 100 shares out.
    w.market_b.simulate_loss(&2_000);

    // 45 XLM buys 50 shares at 0.9 XLM per share.
    let carol = funded(&w, 45 * XLM);
    assert_eq!(w.vault.deposit(&carol, &(45 * XLM)), 50 * XLM);
    assert_eq!(w.vault.state().total_shares, 150 * XLM);
    assert_eq!(w.vault.nav(), 135 * XLM);
    // The first user's claim on the vault is unchanged by the new money.
    assert_eq!(w.vault.position(&user).value, 90 * XLM);
}

#[test]
fn nav_is_idle_plus_every_strategy() {
    let w = world();
    let user = funded(&w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));
    w.vault.allocate(&w.market_a.address, &(30 * XLM));
    w.vault.allocate(&w.market_b.address, &(50 * XLM));

    assert_eq!(w.vault.state().idle, 20 * XLM);
    assert_eq!(w.market_a.total_value(), 30 * XLM);
    assert_eq!(w.market_b.total_value(), 50 * XLM);
    assert_eq!(w.vault.nav(), 100 * XLM);

    w.market_a.simulate_loss(&5_000);
    assert_eq!(w.vault.nav(), 85 * XLM);
}

// ----- curator -------------------------------------------------------------

#[test]
fn buy_cover_charges_the_curator_and_reserves_in_the_pool() {
    let w = world();
    // 50 XLM at 2.5% = 1.25 XLM
    assert_eq!(w.vault.buy_cover(&(50 * XLM)), 12_500_000);
    assert_eq!(w.token.balance(&w.admin), 100 * XLM - 12_500_000);
    assert_eq!(w.pool.cover(&w.vault.address).remaining, 50 * XLM);
    assert_eq!(w.pool.position(&w.alice).premium_claimable, 12_500_000);
}

#[test]
fn close_cover_releases_the_remaining_cover() {
    let w = world();
    let user = demo_allocated(&w);
    w.market_b.simulate_loss(&2_000);
    w.vault.withdraw(&user, &(100 * XLM)); // claims 10 of the 50

    w.vault.close_cover();

    assert_eq!(
        w.pool.cover(&w.vault.address),
        cover_pool::Cover {
            remaining: 0,
            claimed: 10 * XLM
        }
    );
    assert_eq!(w.pool.available_capacity(), 990 * XLM);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn allocate_rejects_unknown_strategies() {
    let w = world();
    let user = funded(&w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));
    w.vault.allocate(&Address::generate(&w.env), &(10 * XLM));
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn allocate_cannot_exceed_idle() {
    let w = world();
    let user = funded(&w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));
    w.vault.allocate(&w.market_a.address, &(100 * XLM + 1));
}

#[test]
fn only_the_curator_allocates_and_buys_cover() {
    let w = world();
    let user = funded(&w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));

    w.env.set_auths(&[]);
    assert!(w
        .vault
        .try_allocate(&w.market_a.address, &(10 * XLM))
        .is_err());
    assert!(w.vault.try_buy_cover(&(10 * XLM)).is_err());
    assert!(w.vault.try_close_cover().is_err());
    assert!(w
        .vault
        .try_add_strategy(&Address::generate(&w.env))
        .is_err());
}

// ----- withdrawals ---------------------------------------------------------

#[test]
fn withdraw_without_loss_pays_exactly_and_makes_no_claim() {
    let w = world();
    let user = demo_allocated(&w);

    assert_eq!(w.vault.withdraw(&user, &(100 * XLM)), 100 * XLM);

    assert_eq!(w.token.balance(&user), 100 * XLM);
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 0);
    assert_eq!(w.pool.position(&w.alice).value, 1_000 * XLM);
    assert_eq!(w.vault.state().total_shares, 0);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.token.balance(&w.vault.address), 0);
}

#[test]
fn withdraw_with_loss_claims_the_shortfall_and_pays_in_full() {
    // The whole demo scenario from docs/PLAN.md section 1.
    let w = world();
    let user = demo_allocated(&w);

    // Step 4: Market B fails and loses 20% (10 XLM).
    w.market_b.simulate_loss(&2_000);
    assert_eq!(w.vault.nav(), 90 * XLM);

    // Step 5: value 90, protected 100, shortfall 10.
    let p = w.vault.position(&user);
    assert_eq!(p.value, 90 * XLM);
    assert_eq!(p.principal, 100 * XLM);
    assert_eq!(p.shortfall, 10 * XLM);

    // Steps 6 and 7.
    assert_eq!(w.vault.withdraw(&user, &(100 * XLM)), 100 * XLM);

    assert_eq!(w.token.balance(&user), 100 * XLM);
    assert_eq!(w.pool.position(&w.alice).value, 990 * XLM);
    assert_eq!(
        w.pool.cover(&w.vault.address),
        cover_pool::Cover {
            remaining: 40 * XLM,
            claimed: 10 * XLM
        }
    );
    // The strategies are drained and the vault holds nothing.
    assert_eq!(w.market_a.total_value(), 0);
    assert_eq!(w.market_b.total_value(), 0);
    assert_eq!(w.vault.state().idle, 0);
    assert_eq!(w.token.balance(&w.vault.address), 0);
    assert_eq!(
        w.vault.position(&user),
        PositionView {
            shares: 0,
            value: 0,
            principal: 0,
            shortfall: 0
        }
    );
}

#[test]
fn partial_withdrawal_claims_a_proportional_shortfall() {
    let w = world();
    let user = demo_allocated(&w);
    w.market_b.simulate_loss(&2_000);

    // Half the shares: worth 45, protected for 50, shortfall 5.
    assert_eq!(w.vault.withdraw(&user, &(50 * XLM)), 50 * XLM);

    assert_eq!(w.token.balance(&user), 50 * XLM);
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 5 * XLM);
    assert_eq!(w.pool.position(&w.alice).value, 995 * XLM);
    let p = w.vault.position(&user);
    assert_eq!(p.shares, 50 * XLM);
    assert_eq!(p.principal, 50 * XLM);
    assert_eq!(p.value, 45 * XLM);
    assert_eq!(p.shortfall, 5 * XLM);

    // The other half later: same again.
    assert_eq!(w.vault.withdraw(&user, &(50 * XLM)), 50 * XLM);
    assert_eq!(w.token.balance(&user), 100 * XLM);
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 10 * XLM);
    assert_eq!(w.pool.position(&w.alice).value, 990 * XLM);
}

#[test]
fn late_depositor_after_a_loss_has_no_shortfall() {
    let w = world();
    let user = demo_allocated(&w);
    w.market_b.simulate_loss(&2_000);

    let carol = funded(&w, 90 * XLM);
    w.vault.deposit(&carol, &(90 * XLM)); // 100 shares at 0.9
    let p = w.vault.position(&carol);
    assert_eq!(p.principal, 90 * XLM);
    assert_eq!(p.value, 90 * XLM);
    assert_eq!(p.shortfall, 0);

    // Carol leaves with exactly her money and no claim.
    assert_eq!(w.vault.withdraw(&carol, &(100 * XLM)), 90 * XLM);
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 0);

    // The original user is still made whole by the pool.
    assert_eq!(w.vault.withdraw(&user, &(100 * XLM)), 100 * XLM);
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 10 * XLM);
}

#[test]
fn withdraw_takes_idle_first_then_strategies_in_order() {
    let w = world();
    let user = funded(&w, 100 * XLM);
    w.vault.deposit(&user, &(100 * XLM));
    w.vault.allocate(&w.market_a.address, &(30 * XLM));
    w.vault.allocate(&w.market_b.address, &(50 * XLM));
    // idle 20, A 30, B 50

    w.vault.withdraw(&user, &(40 * XLM)); // needs 40: idle 20, then 20 from A
    assert_eq!(w.vault.state().idle, 0);
    assert_eq!(w.market_a.total_value(), 10 * XLM);
    assert_eq!(w.market_b.total_value(), 50 * XLM);
    assert_eq!(w.vault.nav(), 60 * XLM);
}

#[test]
fn withdraw_requires_the_depositor_to_sign() {
    let w = world();
    let user = demo_allocated(&w);
    w.env.set_auths(&[]);
    assert!(w.vault.try_withdraw(&user, &(10 * XLM)).is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn cannot_withdraw_more_shares_than_held() {
    let w = world();
    let user = demo_allocated(&w);
    w.vault.withdraw(&user, &(100 * XLM + 1));
}

// ===========================================================================
// Scenarios ported from the Ethereum CoveredVaultWrapper suite, recomputed
// for the MVP (no premium share, no cap, no deductible).
// ===========================================================================

// ----- A. basics -----------------------------------------------------------

#[test]
fn two_depositors_get_shares_for_their_principal() {
    let w = world();
    let u1 = deposit(&w, 600 * XLM);
    let u2 = deposit(&w, 400 * XLM);

    assert_eq!(
        w.vault.position(&u1),
        view(600 * XLM, 600 * XLM, 600 * XLM, 0)
    );
    assert_eq!(
        w.vault.position(&u2),
        view(400 * XLM, 400 * XLM, 400 * XLM, 0)
    );
    assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
    assert_books(&w, &[&u1, &u2], 1_000 * XLM);
}

#[test]
fn full_withdrawal_without_loss_drains_the_strategy_and_leaves_cover_alone() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let user = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    assert_eq!(w.market_a.total_value(), 1_000 * XLM);

    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_000 * XLM);

    assert_eq!(w.token.balance(&user), 1_000 * XLM);
    assert_eq!(w.market_a.total_value(), 0);
    assert_eq!(remaining(&w), 500 * XLM);
    assert_eq!(claimed(&w), 0);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.vault.state(), VaultState::default());
    assert_eq!(w.vault.position(&user), view(0, 0, 0, 0));
}

#[test]
fn partial_withdrawal_without_loss_keeps_the_rest() {
    let w = world();
    let user = deposit(&w, 1_000 * XLM);

    assert_eq!(w.vault.withdraw(&user, &(500 * XLM)), 500 * XLM);

    assert_eq!(
        w.vault.position(&user),
        view(500 * XLM, 500 * XLM, 500 * XLM, 0)
    );
    assert_eq!(w.vault.state().total_shares, 500 * XLM);
    assert_eq!(w.vault.nav(), 500 * XLM);
}

// ----- B. a single loss ----------------------------------------------------

#[test]
fn total_loss_is_paid_entirely_by_the_pool() {
    let w = world();
    buy_cover(&w, 1_000 * XLM);
    let user = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&10_000);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(
        w.vault.position(&user),
        view(1_000 * XLM, 0, 1_000 * XLM, 1_000 * XLM)
    );

    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_000 * XLM);

    assert_eq!(w.token.balance(&user), 1_000 * XLM);
    assert_eq!(
        w.pool.cover(&w.vault.address),
        cover_pool::Cover {
            remaining: 0,
            claimed: 1_000 * XLM
        }
    );
    assert_eq!(w.pool.position(&w.alice).value, 0);
    assert_eq!(w.vault.state(), VaultState::default());
}

#[test]
fn half_loss_is_topped_up_to_principal() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let user = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&5_000);
    assert_eq!(w.vault.position(&user).value, 500 * XLM);

    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_000 * XLM);

    assert_eq!(w.token.balance(&user), 1_000 * XLM);
    assert_eq!(claimed(&w), 500 * XLM);
    assert_eq!(w.vault.nav(), 0);
}

#[test]
fn two_equal_depositors_are_both_made_whole_in_either_order() {
    for u2_first in [false, true] {
        let w = world();
        buy_cover(&w, 500 * XLM);
        let u1 = deposit(&w, 1_000 * XLM);
        let u2 = deposit(&w, 1_000 * XLM);
        allocate_all(&w);
        w.market_a.simulate_loss(&1_000);
        assert_eq!(w.vault.nav(), 1_800 * XLM);

        let (first, second) = if u2_first { (&u2, &u1) } else { (&u1, &u2) };

        assert_eq!(w.vault.position(first).value, 900 * XLM);
        assert_eq!(w.vault.withdraw(first, &(1_000 * XLM)), 1_000 * XLM);
        assert_eq!(w.vault.nav(), 900 * XLM);
        assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
        assert_eq!(claimed(&w), 100 * XLM);

        assert_eq!(w.vault.position(second).value, 900 * XLM);
        assert_eq!(w.vault.withdraw(second, &(1_000 * XLM)), 1_000 * XLM);
        assert_eq!(claimed(&w), 200 * XLM);

        assert_eq!(w.token.balance(&u1), 1_000 * XLM);
        assert_eq!(w.token.balance(&u2), 1_000 * XLM);
        assert_eq!(w.vault.nav(), 0);
        assert_eq!(w.vault.state(), VaultState::default());
        assert_eq!(w.market_a.total_value(), 0);
    }
}

#[test]
fn partial_withdrawals_of_different_sizes_share_the_loss_pro_rata() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let u1 = deposit(&w, 2_000 * XLM);
    let u2 = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&1_000);
    assert_eq!(w.vault.nav(), 2_700 * XLM);

    // u1: 1000 of 2000 shares, worth 900, protected for 1000.
    assert_eq!(w.vault.withdraw(&u1, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(
        w.vault.position(&u1),
        view(1_000 * XLM, 900 * XLM, 1_000 * XLM, 100 * XLM)
    );
    assert_eq!(w.vault.nav(), 1_800 * XLM);
    assert_eq!(w.vault.state().total_shares, 2_000 * XLM);
    assert_eq!(claimed(&w), 100 * XLM);

    // u2: 500 of 1000 shares, worth 450, protected for 500.
    assert_eq!(w.vault.withdraw(&u2, &(500 * XLM)), 500 * XLM);
    assert_eq!(
        w.vault.position(&u2),
        view(500 * XLM, 450 * XLM, 500 * XLM, 50 * XLM)
    );
    assert_eq!(w.vault.state().total_shares, 1_500 * XLM);
    assert_eq!(w.vault.nav(), 1_350 * XLM);
    assert_eq!(claimed(&w), 150 * XLM);

    assert_eq!(
        w.vault.position(&u1),
        view(1_000 * XLM, 900 * XLM, 1_000 * XLM, 100 * XLM)
    );
    assert_books(&w, &[&u1, &u2], 1_500 * XLM);
}

#[test]
fn profit_is_paid_out_without_a_claim() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let user = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    donate(&w, &w.market_a, 200 * XLM);
    assert_eq!(w.vault.nav(), 1_200 * XLM);
    assert_eq!(
        w.vault.position(&user),
        view(1_000 * XLM, 1_200 * XLM, 1_000 * XLM, 0)
    );

    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_200 * XLM);

    assert_eq!(w.token.balance(&user), 1_200 * XLM);
    assert_eq!(claimed(&w), 0);
    assert_eq!(w.vault.position(&user), view(0, 0, 0, 0));
    assert_eq!(w.vault.nav(), 0);
}

// ----- C. many depositors --------------------------------------------------

#[test]
fn five_depositors_keep_the_books_balanced_through_a_loss() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let amounts = [100, 200, 300, 150, 250].map(|a| a * XLM);
    let users = amounts.map(|a| deposit(&w, a));
    let refs: [&Address; 5] = [&users[0], &users[1], &users[2], &users[3], &users[4]];
    assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
    assert_books(&w, &refs, 1_000 * XLM);
    allocate_all(&w);
    assert_books(&w, &refs, 1_000 * XLM);

    assert_eq!(w.vault.withdraw(&users[0], &(50 * XLM)), 50 * XLM);
    assert_eq!(w.vault.state().total_shares, 950 * XLM);
    assert_books(&w, &refs, 950 * XLM);

    // 15% of 950 = 142.5 lost.
    assert_eq!(w.market_a.simulate_loss(&1_500), 1_425_000_000);
    assert_eq!(w.vault.nav(), 8_075_000_000);
    assert_books(&w, &refs, 950 * XLM);

    // u3: 150 shares worth 127.5, protected for 150, shortfall 22.5.
    assert_eq!(
        w.vault.position(&users[3]),
        view(150 * XLM, 1_275_000_000, 150 * XLM, 225_000_000)
    );
    assert_eq!(w.vault.withdraw(&users[3], &(150 * XLM)), 150 * XLM);
    assert_eq!(claimed(&w), 225_000_000);
    assert_eq!(w.vault.state().total_shares, 800 * XLM);
    assert_eq!(w.vault.nav(), 680 * XLM);
    assert_books(&w, &refs, 800 * XLM);
}

#[test]
fn depositors_at_different_entry_prices_are_each_protected_for_their_own_principal() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let ann = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    donate(&w, &w.market_a, 500 * XLM);
    assert_eq!(w.vault.nav(), 1_500 * XLM);

    // Bob pays 1.5 per share.
    let bob = funded(&w, 1_500 * XLM);
    assert_eq!(w.vault.deposit(&bob, &(1_500 * XLM)), 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&1_000);
    assert_eq!(w.vault.nav(), 2_700 * XLM);

    // Ann is still in profit: no claim.
    assert_eq!(
        w.vault.position(&ann),
        view(1_000 * XLM, 1_350 * XLM, 1_000 * XLM, 0)
    );
    assert_eq!(w.vault.withdraw(&ann, &(1_000 * XLM)), 1_350 * XLM);
    assert_eq!(claimed(&w), 0);

    // Bob is 150 under water.
    assert_eq!(
        w.vault.position(&bob),
        view(1_000 * XLM, 1_350 * XLM, 1_500 * XLM, 150 * XLM)
    );
    assert_eq!(w.vault.withdraw(&bob, &(1_000 * XLM)), 1_500 * XLM);
    assert_eq!(claimed(&w), 150 * XLM);
    assert_eq!(w.vault.nav(), 0);
}

/// Ann deposits 1,000, the vault earns 400, Bob deposits 1,000 at 1.4 per
/// share and everything is allocated. NAV 2,400.
fn ann_in_profit_then_bob<'a>(w: &World<'a>) -> (Address, Address) {
    buy_cover(w, 500 * XLM);
    let ann = deposit(w, 1_000 * XLM);
    allocate_all(w);
    donate(w, &w.market_a, 400 * XLM);
    assert_eq!(w.vault.nav(), 1_400 * XLM);
    let bob = funded(w, 1_000 * XLM);
    assert_eq!(w.vault.deposit(&bob, &(1_000 * XLM)), 7_142_857_142);
    allocate_all(w);
    assert_eq!(w.vault.state().total_shares, 17_142_857_142);
    assert_eq!(w.vault.nav(), 2_400 * XLM);
    (ann, bob)
}

#[test]
fn rounding_at_a_fractional_share_price_never_creates_a_claim() {
    let w = world();
    let (ann, bob) = ann_in_profit_then_bob(&w);

    assert_eq!(w.vault.withdraw(&ann, &(1_000 * XLM)), 1_400 * XLM);
    assert_eq!(claimed(&w), 0);
    assert_eq!(w.vault.nav(), 1_000 * XLM);

    // Bob's shares are worth exactly his principal, to the stroop.
    assert_eq!(
        w.vault.position(&bob),
        view(7_142_857_142, 1_000 * XLM, 1_000 * XLM, 0)
    );
    assert_eq!(w.vault.withdraw(&bob, &7_142_857_142), 1_000 * XLM);
    assert_eq!(claimed(&w), 0);
    assert_eq!(w.vault.nav(), 0);
}

#[test]
fn a_loss_can_leave_one_depositor_in_profit_and_insure_the_other() {
    let w = world();
    let (ann, bob) = ann_in_profit_then_bob(&w);
    w.market_a.simulate_loss(&2_500);
    assert_eq!(w.vault.nav(), 1_800 * XLM);

    assert_eq!(w.vault.position(&ann).value, 10_500_000_000);
    assert_eq!(w.vault.withdraw(&ann, &(1_000 * XLM)), 10_500_000_000);
    assert_eq!(claimed(&w), 0);

    assert_eq!(
        w.vault.position(&bob),
        view(7_142_857_142, 750 * XLM, 1_000 * XLM, 250 * XLM)
    );
    assert_eq!(w.vault.withdraw(&bob, &7_142_857_142), 1_000 * XLM);
    assert_eq!(claimed(&w), 250 * XLM);

    // The vault's 1,800 went to the two of them (1,050 + 750); the pool
    // paid the rest.
    assert_eq!(
        w.token.balance(&ann) + w.token.balance(&bob),
        1_800 * XLM + claimed(&w)
    );
    assert_eq!(w.vault.nav(), 0);
}

#[test]
fn late_depositor_through_two_losses_in_either_order() {
    for bob_first in [false, true] {
        let w = world();
        buy_cover(&w, 500 * XLM);
        let ann = deposit(&w, 1_000 * XLM);
        allocate_all(&w);
        w.market_a.simulate_loss(&2_000);
        assert_eq!(w.vault.nav(), 800 * XLM);

        let bob = funded(&w, 1_000 * XLM);
        assert_eq!(w.vault.deposit(&bob, &(1_000 * XLM)), 1_250 * XLM);
        assert_eq!(w.vault.state().total_shares, 2_250 * XLM);
        assert_eq!(w.vault.nav(), 1_800 * XLM);
        assert_eq!(
            w.vault.position(&ann),
            view(1_000 * XLM, 800 * XLM, 1_000 * XLM, 200 * XLM)
        );
        assert_eq!(
            w.vault.position(&bob),
            view(1_250 * XLM, 1_000 * XLM, 1_000 * XLM, 0)
        );

        allocate_all(&w);
        w.market_a.simulate_loss(&1_000);
        assert_eq!(w.vault.nav(), 1_620 * XLM);
        assert_eq!(
            w.vault.position(&ann),
            view(1_000 * XLM, 720 * XLM, 1_000 * XLM, 280 * XLM)
        );
        assert_eq!(
            w.vault.position(&bob),
            view(1_250 * XLM, 900 * XLM, 1_000 * XLM, 100 * XLM)
        );

        if bob_first {
            assert_eq!(w.vault.withdraw(&bob, &(1_250 * XLM)), 1_000 * XLM);
            assert_eq!(claimed(&w), 100 * XLM);
            assert_eq!(w.vault.nav(), 720 * XLM);
            assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
            assert_eq!(w.vault.position(&ann).value, 720 * XLM);
            assert_eq!(w.vault.withdraw(&ann, &(1_000 * XLM)), 1_000 * XLM);
        } else {
            assert_eq!(w.vault.withdraw(&ann, &(1_000 * XLM)), 1_000 * XLM);
            assert_eq!(claimed(&w), 280 * XLM);
            assert_eq!(w.vault.nav(), 900 * XLM);
            assert_eq!(w.vault.state().total_shares, 1_250 * XLM);
            assert_eq!(w.vault.position(&bob).value, 900 * XLM);
            assert_eq!(w.vault.withdraw(&bob, &(1_250 * XLM)), 1_000 * XLM);
        }
        // The two losses were 200 and 180.
        assert_eq!(claimed(&w), 380 * XLM);
        assert_eq!(w.token.balance(&ann), 1_000 * XLM);
        assert_eq!(w.token.balance(&bob), 1_000 * XLM);
        assert_eq!(w.vault.nav(), 0);
    }
}

#[test]
fn repeat_deposits_add_to_one_position() {
    let w = world();
    let user = funded(&w, 2_000 * XLM);
    w.vault.deposit(&user, &(1_000 * XLM));
    w.vault.deposit(&user, &(1_000 * XLM));
    assert_eq!(
        w.vault.position(&user),
        view(2_000 * XLM, 2_000 * XLM, 2_000 * XLM, 0)
    );

    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(
        w.vault.position(&user),
        view(1_000 * XLM, 1_000 * XLM, 1_000 * XLM, 0)
    );
    assert_eq!(w.vault.withdraw(&user, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(w.vault.position(&user), view(0, 0, 0, 0));
    assert_eq!(w.token.balance(&user), 2_000 * XLM);
}

#[test]
fn repeat_deposit_after_a_loss_is_protected_pro_rata() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let user = funded(&w, 2_000 * XLM);
    w.vault.deposit(&user, &(1_000 * XLM));
    allocate_all(&w);
    w.market_a.simulate_loss(&2_000);
    assert_eq!(w.vault.nav(), 800 * XLM);

    // 1,000 more at 0.8 per share.
    assert_eq!(w.vault.deposit(&user, &(1_000 * XLM)), 1_250 * XLM);
    assert_eq!(w.vault.nav(), 1_800 * XLM);
    assert_eq!(
        w.vault.position(&user),
        view(2_250 * XLM, 1_800 * XLM, 2_000 * XLM, 200 * XLM)
    );

    // Half the shares: worth 900, protected for 1000.
    assert_eq!(w.vault.withdraw(&user, &(1_125 * XLM)), 1_000 * XLM);
    assert_eq!(
        w.vault.position(&user),
        view(1_125 * XLM, 900 * XLM, 1_000 * XLM, 100 * XLM)
    );
    assert_eq!(w.vault.nav(), 900 * XLM);
    assert_eq!(claimed(&w), 100 * XLM);

    assert_eq!(w.vault.withdraw(&user, &(1_125 * XLM)), 1_000 * XLM);
    assert_eq!(claimed(&w), 200 * XLM);
    assert_eq!(w.token.balance(&user), 2_000 * XLM);
    assert_eq!(w.vault.nav(), 0);
}

#[test]
fn interleaved_partial_and_full_withdrawals_empty_the_vault() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let ann = deposit(&w, 1_000 * XLM);
    let bob = deposit(&w, 2_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&1_000);
    assert_eq!(w.vault.nav(), 2_700 * XLM);

    assert_eq!(w.vault.withdraw(&ann, &(500 * XLM)), 500 * XLM);
    assert_eq!(w.vault.nav(), 2_250 * XLM);
    assert_eq!(w.vault.state().total_shares, 2_500 * XLM);
    assert_eq!(claimed(&w), 50 * XLM);

    assert_eq!(w.vault.position(&bob).value, 1_800 * XLM);
    assert_eq!(w.vault.withdraw(&bob, &(2_000 * XLM)), 2_000 * XLM);
    assert_eq!(w.vault.nav(), 450 * XLM);
    assert_eq!(w.vault.state().total_shares, 500 * XLM);
    assert_eq!(claimed(&w), 250 * XLM);

    assert_eq!(w.vault.position(&ann).value, 450 * XLM);
    assert_eq!(w.vault.withdraw(&ann, &(500 * XLM)), 500 * XLM);
    assert_eq!(claimed(&w), 300 * XLM);

    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.vault.state(), VaultState::default());
    assert_eq!(w.market_a.total_value(), 0);
    assert_eq!(w.market_b.total_value(), 0);
    assert_eq!(w.token.balance(&w.vault.address), 0);
}

#[test]
fn a_loss_after_one_depositor_left_falls_on_the_one_who_stayed() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let u1 = deposit(&w, 1_000 * XLM);
    let u2 = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    assert_eq!(w.vault.state().idle, 0);

    // u1 leaves before any loss: pulled from the strategy, no claim.
    assert_eq!(w.vault.withdraw(&u1, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(w.market_a.total_value(), 1_000 * XLM);
    assert_eq!(claimed(&w), 0);

    w.market_a.simulate_loss(&2_000);
    assert_eq!(
        w.vault.position(&u2),
        view(1_000 * XLM, 800 * XLM, 1_000 * XLM, 200 * XLM)
    );
    assert_eq!(w.vault.withdraw(&u2, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(claimed(&w), 200 * XLM);
    assert_eq!(w.vault.nav(), 0);
}

// ----- D. cover exhaustion -------------------------------------------------
// When the shortfall exceeds the remaining cover the pool refuses the claim
// and the whole withdrawal fails. Documented limitation: the depositor can
// still leave in pieces the cover can pay for.

#[test]
fn withdrawal_fails_when_the_shortfall_exceeds_the_remaining_cover() {
    let w = world();
    buy_cover(&w, 250 * XLM);
    let u1 = deposit(&w, 200 * XLM);
    let u2 = deposit(&w, 200 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&10_000);
    assert_eq!(w.vault.nav(), 0);

    assert_eq!(w.vault.withdraw(&u1, &(200 * XLM)), 200 * XLM);
    assert_eq!(remaining(&w), 50 * XLM);

    // u2 needs 200 from the pool; only 50 is left.
    assert!(w.vault.try_withdraw(&u2, &(200 * XLM)).is_err());
    assert_eq!(
        w.vault.position(&u2),
        view(200 * XLM, 0, 200 * XLM, 200 * XLM)
    );
    assert_eq!(w.vault.state().total_shares, 200 * XLM);
    assert_eq!(remaining(&w), 50 * XLM);
    assert_eq!(w.token.balance(&u2), 0);

    // A quarter of the position is exactly what the cover can still pay.
    assert_eq!(w.vault.withdraw(&u2, &(50 * XLM)), 50 * XLM);
    assert_eq!(remaining(&w), 0);
    assert_eq!(claimed(&w), 250 * XLM);
    assert_eq!(
        w.vault.position(&u2),
        view(150 * XLM, 0, 150 * XLM, 150 * XLM)
    );

    assert!(w.vault.try_withdraw(&u2, &(150 * XLM)).is_err());
    assert_eq!(
        w.vault.position(&u2),
        view(150 * XLM, 0, 150 * XLM, 150 * XLM)
    );
}

#[test]
fn cover_exhaustion_after_a_partial_loss_leaves_state_unchanged() {
    let w = world();
    buy_cover(&w, 150 * XLM);
    let u1 = deposit(&w, 1_000 * XLM);
    let u2 = deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&1_000);
    assert_eq!(w.vault.nav(), 1_800 * XLM);

    assert_eq!(w.vault.withdraw(&u1, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(remaining(&w), 50 * XLM);
    assert_eq!(w.vault.nav(), 900 * XLM);

    // u2's shortfall is 100; the cover has 50. Nothing moves.
    assert!(w.vault.try_withdraw(&u2, &(1_000 * XLM)).is_err());
    assert_eq!(w.vault.nav(), 900 * XLM);
    assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
    assert_eq!(w.market_a.total_value(), 900 * XLM);
    assert_eq!(
        w.vault.position(&u2),
        view(1_000 * XLM, 900 * XLM, 1_000 * XLM, 100 * XLM)
    );
    assert_eq!(remaining(&w), 50 * XLM);
    assert_eq!(w.token.balance(&u2), 0);

    // Half at a time: shortfall 50, which the cover can pay once.
    assert_eq!(w.vault.withdraw(&u2, &(500 * XLM)), 500 * XLM);
    assert_eq!(remaining(&w), 0);
    assert_eq!(
        w.vault.position(&u2),
        view(500 * XLM, 450 * XLM, 500 * XLM, 50 * XLM)
    );
    assert!(w.vault.try_withdraw(&u2, &(500 * XLM)).is_err());
    assert_eq!(
        w.vault.position(&u2),
        view(500 * XLM, 450 * XLM, 500 * XLM, 50 * XLM)
    );

    // A depositor who is not under water needs no cover at all.
    let carol = funded(&w, 450 * XLM);
    assert_eq!(w.vault.deposit(&carol, &(450 * XLM)), 500 * XLM);
    assert_eq!(w.vault.withdraw(&carol, &(500 * XLM)), 450 * XLM);
    assert_eq!(claimed(&w), 150 * XLM);
    assert_eq!(w.token.balance(&carol), 450 * XLM);
}

// ----- E. rounding, in stroops ---------------------------------------------

#[test]
fn deposit_that_would_mint_no_shares_is_rejected() {
    let w = world();
    // One stroop buys one share; then the vault is worth 10,000,001 stroops.
    let attacker = funded(&w, 1);
    assert_eq!(w.vault.deposit(&attacker, &1), 1);
    donate(&w, &w.market_a, 10_000_000);
    assert_eq!(w.vault.nav(), 10_000_001);

    let victim = funded(&w, 20_000_000);
    assert_eq!(
        w.vault.try_deposit(&victim, &5_000_000),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::ZeroShares as u32
        )))
    );
    assert_eq!(w.token.balance(&victim), 20_000_000);
    assert_eq!(w.vault.state().total_shares, 1);

    assert_eq!(w.vault.deposit(&victim, &15_000_000), 1);
    assert_eq!(w.vault.state().total_shares, 2);
}

#[test]
fn late_depositor_rounding_still_pays_every_stroop_of_principal() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let ann = deposit(&w, 10);
    allocate_all(&w);
    assert_eq!(w.market_a.simulate_loss(&3_000), 3);
    assert_eq!(w.vault.nav(), 7);

    // 5 stroops at 0.7 per share: 7.14 rounds down to 7 shares.
    let bob = funded(&w, 5);
    assert_eq!(w.vault.deposit(&bob, &5), 7);
    assert_eq!(w.vault.state().total_shares, 17);
    assert_eq!(w.vault.nav(), 12);
    assert_eq!(w.vault.position(&bob), view(7, 4, 5, 1));
    assert_eq!(w.vault.position(&ann), view(10, 7, 10, 3));

    assert_eq!(w.vault.withdraw(&bob, &7), 5);
    assert_eq!(claimed(&w), 1);
    assert_eq!(w.vault.nav(), 8);
    assert_eq!(w.vault.state().total_shares, 10);

    assert_eq!(w.vault.withdraw(&ann, &10), 10);
    assert_eq!(claimed(&w), 3);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.token.balance(&bob), 5);
    assert_eq!(w.token.balance(&ann), 10);
}

#[test]
fn withdrawal_that_would_pay_nothing_is_rejected() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let user = funded(&w, 2_000);
    w.vault.deposit(&user, &1_000);
    allocate_all(&w);
    w.market_a.simulate_loss(&2_000);
    assert_eq!(w.vault.deposit(&user, &1_000), 1_250);
    assert_eq!(w.vault.position(&user), view(2_250, 1_800, 2_000, 200));

    // One share is worth 0.8 and protects 0.89: both round to nothing.
    assert_eq!(
        w.vault.try_withdraw(&user, &1),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            Error::InvalidAmount as u32
        )))
    );
    assert_eq!(w.vault.position(&user), view(2_250, 1_800, 2_000, 200));

    // Seven shares: worth 5.6 -> 5, protected 6.22 -> 6.
    assert_eq!(w.vault.withdraw(&user, &7), 6);
    assert_eq!(claimed(&w), 1);
    assert_eq!(w.vault.position(&user).shares, 2_243);
    assert_eq!(w.vault.position(&user).principal, 1_994);
    assert_eq!(w.vault.nav(), 1_795);

    assert_eq!(w.vault.withdraw(&user, &2_243), 1_994);
    assert_eq!(claimed(&w), 200);
    assert_eq!(w.token.balance(&user), 2_000);
    assert_eq!(w.vault.nav(), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn deposit_into_a_drained_vault_is_rejected() {
    let w = world();
    deposit(&w, 1_000 * XLM);
    allocate_all(&w);
    w.market_a.simulate_loss(&10_000);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.vault.state().total_shares, 1_000 * XLM);
    deposit(&w, 100 * XLM);
}

// ----- F. scripted invariants ----------------------------------------------

#[test]
fn books_balance_through_a_scripted_life_of_the_vault() {
    let w = world();
    buy_cover(&w, 500 * XLM);
    let p = deposit(&w, 1_000 * XLM);
    let q = deposit(&w, 500 * XLM);
    // r joins later; until then their position reads as zeros.
    let r = funded(&w, 440 * XLM);
    let users = [&p, &q, &r];
    let mut principal = 1_500 * XLM;
    assert_books(&w, &users, principal);

    w.vault.allocate(&w.market_a.address, &(900 * XLM));
    w.vault.allocate(&w.market_b.address, &(300 * XLM));
    assert_eq!(w.vault.state().idle, 300 * XLM);
    assert_books(&w, &users, principal);

    // First loss: 20% of A, 180.
    assert_eq!(w.market_a.simulate_loss(&2_000), 180 * XLM);
    assert_eq!(w.vault.nav(), 1_320 * XLM);
    assert_books(&w, &users, principal);

    // Partial exits at 0.88 per share.
    assert_eq!(withdraw_checked(&w, &p, 250 * XLM), 250 * XLM);
    principal -= 250 * XLM;
    assert_eq!(w.vault.nav(), 1_100 * XLM);
    assert_eq!(w.vault.state().idle, 80 * XLM);
    assert_books(&w, &users, principal);

    assert_eq!(withdraw_checked(&w, &q, 125 * XLM), 125 * XLM);
    principal -= 125 * XLM;
    assert_eq!(w.vault.nav(), 990 * XLM);
    assert_eq!(w.vault.state().idle, 0);
    assert_eq!(w.market_a.total_value(), 690 * XLM);
    assert_eq!(claimed(&w), 45 * XLM);
    assert_books(&w, &users, principal);

    // Late deposit at 0.88: 440 buys 500 shares.
    assert_eq!(w.vault.deposit(&r, &(440 * XLM)), 500 * XLM);
    principal += 440 * XLM;
    assert_eq!(w.vault.state().total_shares, 1_625 * XLM);
    assert_eq!(w.vault.nav(), 1_430 * XLM);
    assert_books(&w, &users, principal);

    w.vault.allocate(&w.market_b.address, &(350 * XLM));
    assert_eq!(w.market_b.total_value(), 650 * XLM);
    assert_eq!(w.vault.state().idle, 90 * XLM);
    assert_books(&w, &users, principal);

    // Second loss: 20% of B, 130. NAV 1,300 on 1,625 shares: 0.8 per share.
    assert_eq!(w.market_b.simulate_loss(&2_000), 130 * XLM);
    assert_eq!(w.vault.nav(), 1_300 * XLM);
    assert_eq!(
        w.vault.position(&p),
        view(750 * XLM, 600 * XLM, 750 * XLM, 150 * XLM)
    );
    assert_eq!(
        w.vault.position(&q),
        view(375 * XLM, 300 * XLM, 375 * XLM, 75 * XLM)
    );
    assert_eq!(
        w.vault.position(&r),
        view(500 * XLM, 400 * XLM, 440 * XLM, 40 * XLM)
    );
    assert_books(&w, &users, principal);

    // Everyone leaves.
    assert_eq!(withdraw_checked(&w, &p, 750 * XLM), 750 * XLM);
    principal -= 750 * XLM;
    assert_eq!(w.vault.nav(), 700 * XLM);
    assert_books(&w, &users, principal);

    assert_eq!(withdraw_checked(&w, &q, 375 * XLM), 375 * XLM);
    principal -= 375 * XLM;
    assert_eq!(w.vault.nav(), 400 * XLM);
    assert_books(&w, &users, principal);

    assert_eq!(withdraw_checked(&w, &r, 500 * XLM), 440 * XLM);
    principal -= 440 * XLM;
    assert_eq!(principal, 0);
    assert_books(&w, &users, 0);

    // Every stroop lost by the strategies was paid by the pool.
    assert_eq!(claimed(&w), 310 * XLM);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.vault.state(), VaultState::default());
    assert_eq!(w.market_a.total_value(), 0);
    assert_eq!(w.market_b.total_value(), 0);
    assert_eq!(w.token.balance(&w.vault.address), 0);
    assert_eq!(w.token.balance(&p), 1_000 * XLM);
    assert_eq!(w.token.balance(&q), 500 * XLM);
    assert_eq!(w.token.balance(&r), 440 * XLM);
}
