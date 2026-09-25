//! The demo, end to end: docs/PLAN.md section 1, one step per block, every
//! number asserted. scripts/demo_testnet.sh runs the same steps on testnet.
//!
//! A vault loses money on a strategy, and the depositor still withdraws in
//! full because the insurance pool pays the shortfall at withdrawal.

use cover_pool::{Cover, CoverPool, CoverPoolClient};
use covered_vault::{CoveredVault, CoveredVaultClient};
use mock_strategy::{MockStrategy, MockStrategyClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

/// 1 XLM in stroops.
const XLM: i128 = 10_000_000;
/// Stakers must stay at least this many ledgers.
const LOCK_LEDGERS: u32 = 1_000;
/// The pool charges 2.5% of the limit when cover is opened.
const PREMIUM_BPS: u32 = 250;

struct World<'a> {
    env: Env,
    xlm: TokenClient<'a>,
    pool: CoverPoolClient<'a>,
    vault: CoveredVaultClient<'a>,
    market_a: MockStrategyClient<'a>,
    market_b: MockStrategyClient<'a>,
    /// Deploys everything, curates the vault, pays the premium.
    curator: Address,
    /// Stakes XLM in the CoverPool.
    alice: Address,
    /// Deposits XLM into the vault.
    user: Address,
}

/// Deploys the contracts and funds the actors. Nothing has happened yet.
fn deploy<'a>() -> World<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let curator = Address::generate(&env);
    let alice = Address::generate(&env);
    let user = Address::generate(&env);

    // Native XLM is a Stellar Asset Contract; in tests we mint it.
    let xlm_sac = env.register_stellar_asset_contract_v2(curator.clone());
    let xlm = TokenClient::new(&env, &xlm_sac.address());
    let mint = StellarAssetClient::new(&env, &xlm_sac.address());
    mint.mint(&alice, &(1_000 * XLM));
    mint.mint(&user, &(100 * XLM));
    mint.mint(&curator, &(10 * XLM));

    // Step 0 (setup): pool, then vault pointing at it, then whitelist.
    let pool_id = env.register(
        CoverPool,
        (
            curator.clone(),
            xlm_sac.address(),
            LOCK_LEDGERS,
            PREMIUM_BPS,
        ),
    );
    let pool = CoverPoolClient::new(&env, &pool_id);
    let vault_id = env.register(
        CoveredVault,
        (curator.clone(), xlm_sac.address(), pool_id.clone()),
    );
    let vault = CoveredVaultClient::new(&env, &vault_id);
    pool.add_vault(&vault_id);

    // Step 2: two strategies, standing in for two Blend markets.
    let market_a_id = env.register(
        MockStrategy,
        (curator.clone(), vault_id.clone(), xlm_sac.address()),
    );
    let market_b_id = env.register(
        MockStrategy,
        (curator.clone(), vault_id.clone(), xlm_sac.address()),
    );
    vault.add_strategy(&market_a_id);
    vault.add_strategy(&market_b_id);

    World {
        market_a: MockStrategyClient::new(&env, &market_a_id),
        market_b: MockStrategyClient::new(&env, &market_b_id),
        env,
        xlm,
        pool,
        vault,
        curator,
        alice,
        user,
    }
}

/// Steps 0 to 3: Alice stakes, the curator buys cover, the user deposits and
/// the curator allocates 50/50.
fn stake_cover_deposit_allocate(w: &World) {
    // Step 0: Alice stakes 1,000 XLM. Pool capital 1,000.
    assert_eq!(w.pool.stake(&w.alice, &(1_000 * XLM)), 1_000 * XLM);
    assert_eq!(w.pool.state().total_capital, 1_000 * XLM);
    assert_eq!(w.xlm.balance(&w.alice), 0);

    // Step 0: the curator buys 50 XLM of cover for a 2.5% premium (1.25 XLM).
    // The pool reserves 50 for the vault; the premium goes to Alice.
    assert_eq!(w.vault.buy_cover(&(50 * XLM)), 12_500_000);
    assert_eq!(w.xlm.balance(&w.curator), 10 * XLM - 12_500_000);
    assert_eq!(w.pool.cover(&w.vault.address).remaining, 50 * XLM);
    assert_eq!(w.pool.available_capacity(), 950 * XLM);
    assert_eq!(w.pool.position(&w.alice).premium_claimable, 12_500_000);

    // Step 1 and 3: the vault accepts XLM; the user deposits 100.
    assert_eq!(w.vault.deposit(&w.user, &(100 * XLM)), 100 * XLM);
    assert_eq!(w.xlm.balance(&w.user), 0);

    // Step 3: the curator allocates 50 to Market A and 50 to Market B.
    w.vault.allocate(&w.market_a.address, &(50 * XLM));
    w.vault.allocate(&w.market_b.address, &(50 * XLM));
    assert_eq!(w.market_a.total_value(), 50 * XLM);
    assert_eq!(w.market_b.total_value(), 50 * XLM);
    assert_eq!(w.vault.state().idle, 0);
    assert_eq!(w.vault.nav(), 100 * XLM);
}

#[test]
fn demo_market_b_fails_and_the_user_is_made_whole() {
    let w = deploy();
    stake_cover_deposit_allocate(&w);

    // Step 4: Market B fails and loses 20% of its funds (10 XLM). NAV 90.
    assert_eq!(w.market_b.simulate_loss(&2_000), 10 * XLM);
    assert_eq!(w.market_b.total_value(), 40 * XLM);
    assert_eq!(w.vault.nav(), 90 * XLM);

    // Step 5: the user checks their position: value 90, protected 100,
    // shortfall 10.
    let p = w.vault.position(&w.user);
    assert_eq!(p.value, 90 * XLM);
    assert_eq!(p.principal, 100 * XLM);
    assert_eq!(p.shortfall, 10 * XLM);
    // Nothing has been claimed or slashed yet.
    assert_eq!(w.pool.cover(&w.vault.address).claimed, 0);
    assert_eq!(w.pool.position(&w.alice).value, 1_000 * XLM);

    // Step 6: the user withdraws. The vault pulls 90 from the strategies,
    // claims the 10 shortfall from the CoverPool, and the pool slashes its
    // stakers by 10.
    let paid = w.vault.withdraw(&w.user, &(100 * XLM));

    // Step 7: the user receives the full 100 XLM.
    assert_eq!(paid, 100 * XLM);
    assert_eq!(w.xlm.balance(&w.user), 100 * XLM);

    // Alice's stake 1,000 -> 990; remaining cover 50 -> 40.
    assert_eq!(w.pool.position(&w.alice).value, 990 * XLM);
    assert_eq!(w.pool.state().total_capital, 990 * XLM);
    assert_eq!(
        w.pool.cover(&w.vault.address),
        Cover {
            remaining: 40 * XLM,
            claimed: 10 * XLM
        }
    );
    assert_eq!(w.pool.state().coverage_written, 40 * XLM);

    // Where every stroop ended up.
    assert_eq!(w.market_a.total_value(), 0);
    assert_eq!(w.market_b.total_value(), 0);
    assert_eq!(w.vault.nav(), 0);
    assert_eq!(w.xlm.balance(&w.vault.address), 0);
    // Pool holds Alice's slashed capital plus her unclaimed premium.
    assert_eq!(w.xlm.balance(&w.pool.address), 990 * XLM + 12_500_000);
    // The "market" that failed walked off with 10 XLM (the curator, in the
    // mock).
    assert_eq!(w.xlm.balance(&w.curator), 10 * XLM - 12_500_000 + 10 * XLM);

    // Epilogue: Alice collects her premium, and after the lock she unstakes
    // once the curator closes the cover that her capital still backs.
    assert_eq!(w.pool.claim_premium(&w.alice), 12_500_000);
    w.env
        .ledger()
        .with_mut(|l| l.sequence_number += LOCK_LEDGERS);
    assert!(w.pool.try_unstake(&w.alice, &(1_000 * XLM)).is_err()); // 40 still reserved
    w.vault.close_cover();
    assert_eq!(w.pool.unstake(&w.alice, &(1_000 * XLM)), 990 * XLM);
    assert_eq!(w.xlm.balance(&w.alice), 990 * XLM + 12_500_000);
}

#[test]
fn quiet_path_no_loss_no_claim_no_slashing() {
    let w = deploy();
    stake_cover_deposit_allocate(&w);

    // No loss: value 100, protected 100, shortfall 0.
    let p = w.vault.position(&w.user);
    assert_eq!(p.value, 100 * XLM);
    assert_eq!(p.principal, 100 * XLM);
    assert_eq!(p.shortfall, 0);

    // The user withdraws exactly what they put in.
    assert_eq!(w.vault.withdraw(&w.user, &(100 * XLM)), 100 * XLM);
    assert_eq!(w.xlm.balance(&w.user), 100 * XLM);

    // No claim, no slashing: the cover is untouched and Alice is whole.
    assert_eq!(
        w.pool.cover(&w.vault.address),
        Cover {
            remaining: 50 * XLM,
            claimed: 0
        }
    );
    assert_eq!(w.pool.position(&w.alice).value, 1_000 * XLM);
    assert_eq!(w.pool.state().claims_paid, 0);
    assert_eq!(w.vault.nav(), 0);
}
