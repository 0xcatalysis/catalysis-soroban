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
