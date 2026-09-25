#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

/// 1 XLM in stroops.
const XLM: i128 = 10_000_000;

struct Setup<'a> {
    env: Env,
    strategy: MockStrategyClient<'a>,
    token: TokenClient<'a>,
    admin: Address,
    vault: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let asset = sac.address();

    let id = env.register(MockStrategy, (admin.clone(), vault.clone(), asset.clone()));
    Setup {
        strategy: MockStrategyClient::new(&env, &id),
        token: TokenClient::new(&env, &asset),
        env,
        admin,
        vault,
    }
}

/// The vault funds a strategy by plain transfer; in tests we mint directly.
fn fund(s: &Setup, amount: i128) {
    StellarAssetClient::new(&s.env, &s.token.address).mint(&s.strategy.address, &amount);
}

#[test]
fn total_value_is_the_token_balance() {
    let s = setup();
    assert_eq!(s.strategy.total_value(), 0);
    fund(&s, 50 * XLM);
    assert_eq!(s.strategy.total_value(), 50 * XLM);
}

#[test]
fn withdraw_moves_funds_to_the_vault() {
    let s = setup();
    fund(&s, 50 * XLM);

    s.strategy.withdraw(&(20 * XLM));

    assert_eq!(s.token.balance(&s.vault), 20 * XLM);
    assert_eq!(s.strategy.total_value(), 30 * XLM);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn withdraw_more_than_held_is_rejected() {
    let s = setup();
    fund(&s, 50 * XLM);
    s.strategy.withdraw(&(50 * XLM + 1));
}

#[test]
fn withdraw_requires_the_vault_to_sign() {
    let s = setup();
    fund(&s, 50 * XLM);
    s.env.set_auths(&[]);
    assert!(s.strategy.try_withdraw(&XLM).is_err());
}

#[test]
fn simulate_loss_removes_a_fraction_to_the_admin() {
    // Demo step 4: Market B loses 20% of its 50 XLM.
    let s = setup();
    fund(&s, 50 * XLM);

    assert_eq!(s.strategy.simulate_loss(&2_000), 10 * XLM);

    assert_eq!(s.strategy.total_value(), 40 * XLM);
    assert_eq!(s.token.balance(&s.admin), 10 * XLM);
    // What is left can still be withdrawn in full.
    s.strategy.withdraw(&(40 * XLM));
    assert_eq!(s.token.balance(&s.vault), 40 * XLM);
}

#[test]
fn simulate_loss_of_everything() {
    let s = setup();
    fund(&s, 50 * XLM);
    assert_eq!(s.strategy.simulate_loss(&10_000), 50 * XLM);
    assert_eq!(s.strategy.total_value(), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn simulate_loss_above_100_percent_is_rejected() {
    let s = setup();
    fund(&s, 50 * XLM);
    s.strategy.simulate_loss(&10_001);
}

#[test]
fn only_the_admin_can_simulate_a_loss() {
    let s = setup();
    fund(&s, 50 * XLM);
    s.env.set_auths(&[]);
    assert!(s.strategy.try_simulate_loss(&1_000).is_err());
}
