#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events},
    token, vec, Address, Env, IntoVal, Map, Symbol, Val, Vec,
};

use crate::{RewardPool, RewardPoolClient};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (Env, RewardPoolClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();

    // Fixed: Passing the contract type first, and empty constructor args second
    let contract_id = env.register(RewardPool, ());

    let client = RewardPoolClient::new(&env, &contract_id);
    (env, client)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_initialize_success() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    // Initialize the contract
    client.initialize(&admin, &token);

    // Verify event was emitted
    assert_eq!(env.events().all().len(), 1);
}

#[test]
#[should_panic(expected = "Already initialized")]
fn test_initialize_twice_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    // First initialization should succeed
    client.initialize(&admin, &token);

    // Second initialization should panic
    client.initialize(&admin, &token);
}

#[test]
#[should_panic(expected = "Error(Auth, InvalidAction)")]
fn test_initialize_without_auth_panics() {
    let env = Env::default();
    let contract_id = env.register(RewardPool, ());
    let client = RewardPoolClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    // Try to initialize without mocking auths - should panic
    client.initialize(&admin, &token);
}

#[test]
fn test_initialize_with_proper_auth() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    // Initialize with proper authentication (mocked by env.mock_all_auths())
    client.initialize(&admin, &token);

    // Verify event was emitted
    assert_eq!(env.events().all().len(), 1);
}

// ── add_approved_spender Tests ────────────────────────────────────────────────

#[test]
fn test_add_approved_spender_success() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let spender = Address::generate(&env);

    // Initialize the contract
    client.initialize(&admin, &token);

    // Add approved spender - should succeed without panic
    client.add_approved_spender(&admin, &spender);

    // assert event emitted
    let empty_data: Map<(), ()> = Map::new(&env);
    let event = vec![
        &env,
        (
            client.address,
            (Symbol::new(&env, "spender_added"), spender).into_val(&env),
            empty_data.into_val(&env),
        ),
    ];

    assert_eq!(env.events().all(), event)
}

#[test]
#[should_panic(expected = "Not initialized")]
fn test_add_approved_spender_not_initialized() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);

    // Try to add spender without initializing - should panic
    client.add_approved_spender(&admin, &spender);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_add_approved_spender_wrong_admin() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let wrong_admin = Address::generate(&env);
    let token = Address::generate(&env);
    let spender = Address::generate(&env);

    // Initialize the contract
    client.initialize(&admin, &token);

    // Try to add spender with wrong admin - should panic
    client.add_approved_spender(&wrong_admin, &spender);
}

#[test]
fn test_add_multiple_approved_spenders() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let spender1 = Address::generate(&env);
    let spender2 = Address::generate(&env);

    // Initialize the contract
    client.initialize(&admin, &token);

    // Add multiple spenders - should succeed without panic
    client.add_approved_spender(&admin, &spender1);
    client.add_approved_spender(&admin, &spender2);
}

#[test]
fn test_add_same_spender_twice() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let spender = Address::generate(&env);

    // Initialize the contract
    client.initialize(&admin, &token);

    // Add same spender twice (should not panic, just overwrite)
    client.add_approved_spender(&admin, &spender);
    client.add_approved_spender(&admin, &spender);
}

#[test]
#[should_panic(expected = "Max spenders reached")]
fn test_add_approved_spender_respects_max_limit() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    client.initialize(&admin, &token);

    for _ in 0..crate::MAX_SPENDERS {
        let spender = Address::generate(&env);
        client.add_approved_spender(&admin, &spender);
    }

    let overflow_spender = Address::generate(&env);
    client.add_approved_spender(&admin, &overflow_spender);
}

#[test]
fn test_token_decimals_is_exposed() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);

    client.initialize(&admin, &token);

    assert_eq!(client.token_decimals(), crate::REWARD_TOKEN_DECIMALS);
}

// ── distribute_reward Tests ───────────────────────────────────────────────────

#[test]
fn test_distribute_reward_success() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let learner = Address::generate(&env);

    // Create and register a mock token contract
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    // Whitelist the spender
    client.add_approved_spender(&admin, &spender);

    // Mint tokens to the reward pool contract
    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&client.address, &1000);

    // Distribute reward
    client.distribute_reward(&spender, &learner, &100);

    // Check events immediately after distribute_reward
    let last_event = env.events().all().last().unwrap();

    let mut data_map = Map::new(&env);
    data_map.set(Symbol::new(&env, "amount"), 100i128);
    let expected_event: (Address, Vec<Val>, Val) = (
        client.address,
        (Symbol::new(&env, "reward_distributed"), &spender, &learner).into_val(&env),
        data_map.into_val(&env),
    );

    // Verify events match
    assert_eq!(
        vec![&env, last_event.clone()],
        vec![&env, expected_event.clone()]
    );

    // Verify learner received tokens
    let learner_balance = token_client.balance(&learner);
    assert_eq!(learner_balance, 100);
}

#[test]
#[should_panic(expected = "Amount must be positive")]
fn test_distribute_reward_zero_amount() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());
    client.add_approved_spender(&admin, &spender);

    // Try to distribute zero amount - should panic
    client.distribute_reward(&spender, &learner, &0);
}

#[test]
#[should_panic(expected = "Amount must be positive")]
fn test_distribute_reward_negative_amount() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());
    client.add_approved_spender(&admin, &spender);

    // Try to distribute negative amount - should panic
    client.distribute_reward(&spender, &learner, &-100);
}

#[test]
#[should_panic(expected = "Caller is not an authorized spender")]
fn test_distribute_reward_unauthorized_spender() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let unauthorized_spender = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());

    // Try to distribute without being whitelisted - should panic
    client.distribute_reward(&unauthorized_spender, &learner, &100);
}

#[test]
#[should_panic(expected = "Not initialized")]
fn test_distribute_reward_not_initialized() {
    let (env, client) = setup();
    let spender = Address::generate(&env);
    let learner = Address::generate(&env);

    // Try to distribute without initializing - should panic
    client.distribute_reward(&spender, &learner, &100);
}

#[test]
fn test_distribute_reward_multiple_times() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender = Address::generate(&env);
    let learner1 = Address::generate(&env);
    let learner2 = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());
    client.add_approved_spender(&admin, &spender);

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&client.address, &1000);

    // Distribute to multiple learners
    client.distribute_reward(&spender, &learner1, &100);
    client.distribute_reward(&spender, &learner2, &200);

    assert_eq!(token_client.balance(&learner1), 100);
    assert_eq!(token_client.balance(&learner2), 200);
}

#[test]
fn test_distribute_reward_multiple_spenders() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let spender1 = Address::generate(&env);
    let spender2 = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());
    client.add_approved_spender(&admin, &spender1);
    client.add_approved_spender(&admin, &spender2);

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&client.address, &1000);

    // Both spenders can distribute
    client.distribute_reward(&spender1, &learner, &100);
    client.distribute_reward(&spender2, &learner, &50);

    assert_eq!(token_client.balance(&learner), 150);
}

// ── fund_pool Tests ───────────────────────────────────────────────────────────

#[test]
fn test_fund_pool_success() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let donor = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    // Mint tokens to the donor
    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&donor, &1000);

    // Fund the pool
    client.fund_pool(&donor, &500);

    // Verify donor's balance decreased
    assert_eq!(token_client.balance(&donor), 500);

    // Verify contract's balance increased
    assert_eq!(token_client.balance(&client.address), 500);
}

#[test]
fn test_fund_pool_emits_event() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let donor = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&donor, &1000);

    client.fund_pool(&donor, &300);

    // Verify event was emitted
    let last_event = env.events().all().last().unwrap();

    let mut data_map = Map::new(&env);
    data_map.set(Symbol::new(&env, "amount"), 300i128);
    let expected_event: (Address, Vec<Val>, Val) = (
        client.address,
        (Symbol::new(&env, "pool_funded"), &donor).into_val(&env),
        data_map.into_val(&env),
    );

    assert_eq!(
        vec![&env, last_event.clone()],
        vec![&env, expected_event.clone()]
    );
}

#[test]
#[should_panic(expected = "Not initialized")]
fn test_fund_pool_not_initialized() {
    let (env, client) = setup();
    let donor = Address::generate(&env);

    // Try to fund without initializing - should panic
    client.fund_pool(&donor, &100);
}

#[test]
fn test_fund_pool_multiple_donors() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let donor1 = Address::generate(&env);
    let donor2 = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&donor1, &1000);
    token_client.mint(&donor2, &1000);

    // Multiple donors fund the pool
    client.fund_pool(&donor1, &500);
    client.fund_pool(&donor2, &300);

    // Verify balances
    assert_eq!(token_client.balance(&donor1), 500);
    assert_eq!(token_client.balance(&donor2), 700);
    assert_eq!(token_client.balance(&client.address), 800);
}

#[test]
fn test_fund_pool_multiple_times() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let donor = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&donor, &2000);

    // Donor funds multiple times
    client.fund_pool(&donor, &500);
    client.fund_pool(&donor, &300);
    client.fund_pool(&donor, &200);

    // Verify balances
    assert_eq!(token_client.balance(&donor), 1000);
    assert_eq!(token_client.balance(&client.address), 1000);
}

#[test]
fn test_fund_pool_zero_amount() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let donor = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    client.initialize(&admin, &token_id.address());

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&donor, &1000);

    // Fund with zero amount (should succeed)
    client.fund_pool(&donor, &0);

    // Verify balances unchanged
    assert_eq!(token_client.balance(&donor), 1000);
    assert_eq!(token_client.balance(&client.address), 0);
}

// ── emergency_sweep Tests ─────────────────────────────────────────────────────

#[test]
fn test_emergency_sweep_success() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let recovery_wallet = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    // Fund the pool with tokens
    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&client.address, &1000);

    // Verify initial balance
    assert_eq!(token_client.balance(&client.address), 1000);
    assert_eq!(token_client.balance(&recovery_wallet), 0);

    // Perform emergency sweep
    client.emergency_sweep(&admin, &recovery_wallet);

    // Verify contract balance is 0
    assert_eq!(token_client.balance(&client.address), 0);

    // Verify recovery wallet received full balance
    assert_eq!(token_client.balance(&recovery_wallet), 1000);
}

#[test]
#[should_panic(expected = "Not initialized")]
fn test_emergency_sweep_not_initialized() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let recovery_wallet = Address::generate(&env);

    // Try to sweep without initializing - should panic
    client.emergency_sweep(&admin, &recovery_wallet);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_emergency_sweep_wrong_admin() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let wrong_admin = Address::generate(&env);
    let recovery_wallet = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    // Try to sweep with wrong admin - should panic
    client.emergency_sweep(&wrong_admin, &recovery_wallet);
}

#[test]
fn test_emergency_sweep_zero_balance() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let recovery_wallet = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    let token_client = token::StellarAssetClient::new(&env, &token_id.address());

    // Verify initial balance is 0
    assert_eq!(token_client.balance(&client.address), 0);

    // Perform emergency sweep with zero balance (should succeed)
    client.emergency_sweep(&admin, &recovery_wallet);

    // Verify balances remain 0
    assert_eq!(token_client.balance(&client.address), 0);
    assert_eq!(token_client.balance(&recovery_wallet), 0);
}

#[test]
fn test_emergency_sweep_large_balance() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let recovery_wallet = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());

    // Initialize the reward pool
    client.initialize(&admin, &token_id.address());

    // Fund with large amount
    let token_client = token::StellarAssetClient::new(&env, &token_id.address());
    token_client.mint(&client.address, &1_000_000);

    // Perform emergency sweep
    client.emergency_sweep(&admin, &recovery_wallet);

    // Verify full balance transferred
    assert_eq!(token_client.balance(&client.address), 0);
    assert_eq!(token_client.balance(&recovery_wallet), 1_000_000);
}

// ── Storage versioning & migrations ──────────────────────────────────────────

/// A freshly deployed contract reports version 0 (no Version key set yet).
#[test]
fn test_reward_pool_contract_version_initial_zero() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    client.initialize(&admin, &token);

    assert_eq!(client.contract_version(), 0);
}

/// After `migrate()` the stored version equals the compiled VERSION constant.
#[test]
fn test_reward_pool_migrate_v0_to_v1_sets_version() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    client.initialize(&admin, &token);

    assert_eq!(client.contract_version(), 0);
    client.migrate(&admin);
    assert_eq!(client.contract_version(), crate::VERSION);
}

/// `migrate()` called twice panics with "Already at current version".
#[test]
#[should_panic(expected = "Already at current version")]
fn test_reward_pool_migrate_twice_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    client.initialize(&admin, &token);

    client.migrate(&admin);
    client.migrate(&admin);
}

/// Non-admin cannot call `migrate()`.
#[test]
#[should_panic(expected = "Unauthorized")]
fn test_reward_pool_migrate_unauthorized_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let token = Address::generate(&env);
    client.initialize(&admin, &token);

    client.migrate(&attacker);
}

/// Approved spender entries survive migration (v0 → v1).
#[test]
fn test_reward_pool_migrate_v0_to_v1_preserves_spender_count() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let token = Address::generate(&env);
    let spender = Address::generate(&env);
    client.initialize(&admin, &token);
    client.add_approved_spender(&admin, &spender);

    assert_eq!(client.contract_version(), 0);
    client.migrate(&admin);
    assert_eq!(client.contract_version(), crate::VERSION);

    // The spender entry must still be present — distribute_reward would panic
    // otherwise (tested indirectly via estimated_storage_footprint).
    assert_eq!(client.estimated_storage_footprint(), 1);
}
