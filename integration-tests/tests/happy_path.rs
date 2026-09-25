//! End-to-end happy path across all four contracts:
//! stake → buy cover → bad debt socialized in Blend → permissionless payout
//! → stakers slashed pro rata → unstake.

use blend_adapter::BlendAdapter;
use cover_pool::{CoverPool, CoverPoolClient};
use mock_blend_pool::{MockBlendPool, MockBlendPoolClient, RATE_ONE};
use policy::{PolicyContract, PolicyContractClient, Status};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

/// 1 XLM in stroops.
const XLM: i128 = 10_000_000;
const LOCK_LEDGERS: u32 = 1_000;
const DURATION_LEDGERS: u32 = 5_000;
const PREMIUM_BPS: u32 = 250; // 2.5% of limit
const TRIGGER_BPS: u32 = 500; // pays once the covered rate falls 5%

struct World<'a> {
    env: Env,
    xlm: TokenClient<'a>,
    pool: CoverPoolClient<'a>,
    policy: PolicyContractClient<'a>,
    blend: MockBlendPoolClient<'a>,
    /// The Blend reserve asset the covered vault has supplied.
    reserve_asset: Address,
    alice: Address,
    bob: Address,
    vault: Address,
}

fn world<'a>() -> World<'a> {
    let env = Env::default();
    env.mock_all_auths_allowing_non_root_auth();

    let admin = Address::generate(&env);
    let xlm_sac = env.register_stellar_asset_contract_v2(admin.clone());
    let xlm = TokenClient::new(&env, &xlm_sac.address());
    let mint = StellarAssetClient::new(&env, &xlm_sac.address());

    let blend_id = env.register(MockBlendPool, (admin.clone(),));
    let adapter_id = env.register(BlendAdapter, ());
    let pool_id = env.register(
        CoverPool,
        (admin.clone(), xlm.address.clone(), LOCK_LEDGERS),
    );
    let policy_id = env.register(
        PolicyContract,
        (
            admin.clone(),
            pool_id.clone(),
            adapter_id,
            PREMIUM_BPS,
            TRIGGER_BPS,
            DURATION_LEDGERS,
        ),
    );
    let pool = CoverPoolClient::new(&env, &pool_id);
    pool.set_policy_contract(&policy_id);

    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let vault = Address::generate(&env);
    mint.mint(&alice, &(60_000 * XLM));
    mint.mint(&bob, &(40_000 * XLM));
    mint.mint(&vault, &(1_000 * XLM));

    World {
        policy: PolicyContractClient::new(&env, &policy_id),
        blend: MockBlendPoolClient::new(&env, &blend_id),
        reserve_asset: Address::generate(&env),
        pool,
        xlm,
        alice,
        bob,
        vault,
        env,
    }
}

fn advance(env: &Env, ledgers: u32) {
    env.ledger().with_mut(|l| l.sequence_number += ledgers);
}

#[test]
fn bad_debt_event_pays_the_vault_and_slashes_stakers_pro_rata() {
    let w = world();

    // 1. Stakers commit capital: 100,000 XLM of capacity.
    w.pool.stake(&w.alice, &(60_000 * XLM));
    w.pool.stake(&w.bob, &(40_000 * XLM));
    assert_eq!(w.pool.state().total_capital, 100_000 * XLM);

    // 2. A vault with 50,000 supplied to a Blend reserve buys 20,000 of cover.
    let id = w.policy.issue(
        &w.vault,
        &w.vault,
        &w.blend.address,
        &w.reserve_asset,
        &(50_000 * XLM),
        &(20_000 * XLM),
    );
    assert_eq!(w.pool.state().coverage_written, 20_000 * XLM);
    assert_eq!(w.pool.available_capacity(), 80_000 * XLM);
    // Premium: 2.5% of the limit, shared 60/40 between the stakers.
    assert_eq!(w.policy.get(&id).premium, 500 * XLM);
    assert_eq!(w.pool.position(&w.alice).premium_claimable, 300 * XLM);
    assert_eq!(w.pool.position(&w.bob).premium_claimable, 200 * XLM);

    // 3. A YieldBlox-shaped event: Blend socializes bad debt and the
    //    reserve's b_rate drops 12%.
    advance(&w.env, 100);
    w.blend.set_b_rate(&w.reserve_asset, &(RATE_ONE * 88 / 100));
    let c = w.policy.check(&id);
    assert!(c.payable);
    assert_eq!(c.decline_bps, 1_200);

    // 4. Anyone can settle. The vault's 12% loss on 50,000 is paid in full.
    let vault_before = w.xlm.balance(&w.vault);
    let keeper = Address::generate(&w.env);
    let _ = keeper; // settle takes no caller: it is permissionless
    let payout = w.policy.settle(&id);
    assert_eq!(payout, 6_000 * XLM);
    assert_eq!(w.xlm.balance(&w.vault) - vault_before, 6_000 * XLM);
    assert_eq!(w.policy.get(&id).status, Status::Paid);

    // 5. Stakers are slashed 6% each, in exact proportion.
    assert_eq!(w.pool.state().total_capital, 94_000 * XLM);
    assert_eq!(w.pool.state().coverage_written, 0);
    assert_eq!(w.pool.position(&w.alice).value, 56_400 * XLM);
    assert_eq!(w.pool.position(&w.bob).value, 37_600 * XLM);

    // 6. After the lock, stakers exit with what is left plus their premium.
    advance(&w.env, LOCK_LEDGERS);
    let shares = w.pool.position(&w.alice).shares;
    assert_eq!(w.pool.unstake(&w.alice, &shares), 56_400 * XLM);
    assert_eq!(w.pool.claim_premium(&w.alice), 300 * XLM);
    assert_eq!(w.xlm.balance(&w.alice), 56_700 * XLM);
}

#[test]
fn quiet_term_expires_and_stakers_keep_capital_plus_premium() {
    let w = world();
    w.pool.stake(&w.alice, &(60_000 * XLM));
    w.pool.stake(&w.bob, &(40_000 * XLM));
    let id = w.policy.issue(
        &w.vault,
        &w.vault,
        &w.blend.address,
        &w.reserve_asset,
        &(50_000 * XLM),
        &(20_000 * XLM),
    );

    // Nothing happens in Blend: the policy is not payable.
    assert!(!w.policy.check(&id).payable);
    assert!(w.policy.try_settle(&id).is_err());

    // Term ends; anyone expires it and the capacity is released.
    advance(&w.env, DURATION_LEDGERS + 1);
    w.policy.expire(&id);
    assert_eq!(w.policy.get(&id).status, Status::Expired);
    assert_eq!(w.pool.state().coverage_written, 0);

    let shares = w.pool.position(&w.bob).shares;
    assert_eq!(w.pool.unstake(&w.bob, &shares), 40_000 * XLM);
    assert_eq!(w.pool.claim_premium(&w.bob), 200 * XLM);
    assert_eq!(w.xlm.balance(&w.bob), 40_200 * XLM);
}

#[test]
fn small_rate_wobble_below_trigger_does_not_pay() {
    let w = world();
    w.pool.stake(&w.alice, &(60_000 * XLM));
    let id = w.policy.issue(
        &w.vault,
        &w.vault,
        &w.blend.address,
        &w.reserve_asset,
        &(50_000 * XLM),
        &(20_000 * XLM),
    );

    // A 1% dip is under the 5% trigger.
    w.blend.set_b_rate(&w.reserve_asset, &(RATE_ONE * 99 / 100));
    let c = w.policy.check(&id);
    assert_eq!(c.decline_bps, 100);
    assert!(!c.payable);
    assert!(w.policy.try_settle(&id).is_err());
}
