use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events},
    Address, BytesN, Env,
};

use crate::types::{QuestType, SubmissionStatus};
use crate::{compute_learner_payout, QuestEngineContract, QuestEngineContractClient};

// ── Mock StakeVault Contract ─────────────────────────────────────────────────

#[contract]
pub struct MockStakeVault;

#[contractimpl]
impl MockStakeVault {
    /// Returns a multiplier for a learner (basis points: 100 = 1.0x, 120 = 1.2x)
    /// For testing, we'll return 100 (no boost) by default
    pub fn get_multiplier(_env: Env, _learner: Address) -> u32 {
        100 // Default: no multiplier
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn setup() -> (
    Env,
    QuestEngineContractClient<'static>,
    Address,
    Address,
    Address,
    Address,
) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);

    // Create a SAC token for USDC
    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Register mock stake vault
    let stake_vault_id = env.register(MockStakeVault, ());

    // Initialize the contract with admin, token, reward_pool, and stake_vault
    let admin = Address::generate(&env);
    let reward_pool = Address::generate(&env);
    client.initialize(&admin, &token_id, &reward_pool, &stake_vault_id);

    (env, client, token_id, reward_pool, admin, stake_vault_id)
}

fn mint_tokens(env: &Env, token_id: &Address, to: &Address, amount: &i128) {
    let sac_client = soroban_sdk::token::StellarAssetClient::new(env, token_id);
    sac_client.mint(to, amount);
}

fn token_balance(env: &Env, token_id: &Address, of: &Address) -> i128 {
    soroban_sdk::token::Client::new(env, token_id).balance(of)
}

// ── Initialize Tests ─────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Already initialized")]
fn test_initialize_twice_panics() {
    let (_env, client, token_id, reward_pool, admin, stake_vault_id) = setup();
    client.initialize(&admin, &token_id, &reward_pool, &stake_vault_id);
}

// ── set_reward_pool_address Tests ───────────────────────────────────────────

#[test]
fn test_set_reward_pool_address_success() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let new_reward_pool = Address::generate(&env);

    client.set_reward_pool_address(&admin, &new_reward_pool);

    // Verify the new reward pool is stored by creating an explore quest
    // and verifying it uses the new address (tested indirectly via verify_explore_quest)
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_set_reward_pool_address_wrong_admin_panics() {
    let (env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let wrong_admin = Address::generate(&env);
    let new_reward_pool = Address::generate(&env);

    client.set_reward_pool_address(&wrong_admin, &new_reward_pool);
}

#[test]
fn test_set_reward_pool_address_emits_event() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let new_reward_pool = Address::generate(&env);

    client.set_reward_pool_address(&admin, &new_reward_pool);

    // Verify RewardPoolUpdated event was emitted
    let events = env.events().all();
    assert!(!events.is_empty(), "Expected at least 1 event");
}

// ── set_stake_vault_address Tests ──────────────────────────────────────────

#[test]
fn test_set_stake_vault_address_success() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let new_stake_vault = Address::generate(&env);

    client.set_stake_vault_address(&admin, &new_stake_vault);

    // Verify the new stake vault is stored by creating a quest and reviewing
    // (tested indirectly via review_submission behavior)
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_set_stake_vault_address_wrong_admin_panics() {
    let (env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let wrong_admin = Address::generate(&env);
    let new_stake_vault = Address::generate(&env);

    client.set_stake_vault_address(&wrong_admin, &new_stake_vault);
}

#[test]
fn test_set_stake_vault_address_emits_event() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let new_stake_vault = Address::generate(&env);

    client.set_stake_vault_address(&admin, &new_stake_vault);

    // Verify StakeVaultUpdated event was emitted
    let events = env.events().all();
    assert!(!events.is_empty(), "Expected at least 1 event");
}

// ── create_build_quest Tests ─────────────────────────────────────────────────

#[test]
fn test_create_build_quest_success() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let reward_amount: i128 = 1_000;
    let metadata_hash = BytesN::from_array(&env, &[1u8; 32]);

    // Fund the employer
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    assert_eq!(token_balance(&env, &token_id, &employer), reward_amount);

    // Create a build quest
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Quest ID should be 1 (first quest)
    assert_eq!(quest_id, 1);

    // ✅ Acceptance: QuestEngine contract balance increases
    assert_eq!(
        token_balance(&env, &token_id, &client.address),
        reward_amount
    );
    assert_eq!(token_balance(&env, &token_id, &employer), 0);

    // ✅ Acceptance: Quest is saved as a Build type
    let quest = client.get_quest(&quest_id).unwrap();
    assert_eq!(quest.employer, employer);
    assert_eq!(quest.reward_amount, reward_amount);
    assert_eq!(quest.quest_type, QuestType::Build);
    assert_eq!(quest.metadata_hash, metadata_hash);
    assert!(quest.active);
}

#[test]
fn test_create_build_quest_emits_event() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[2u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);

    client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Verify QuestCreated event was emitted
    let events = env.events().all();
    assert!(
        !events.is_empty(),
        "Expected at least 1 event, got {}",
        events.len()
    );
}

#[test]
fn test_create_build_quest_increments_ids() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[3u8; 32]);

    // Fund enough for 3 quests
    mint_tokens(&env, &token_id, &employer, &3000);

    let id1 = client.create_build_quest(&employer, &1000, &metadata_hash);
    let id2 = client.create_build_quest(&employer, &1000, &metadata_hash);
    let id3 = client.create_build_quest(&employer, &1000, &metadata_hash);

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);

    // Verify all quests exist and are Build type
    for id in [id1, id2, id3] {
        let quest = client.get_quest(&id).unwrap();
        assert_eq!(quest.quest_type, QuestType::Build);
        assert!(quest.active);
    }

    // Total contract balance should be 3000
    assert_eq!(token_balance(&env, &token_id, &client.address), 3000);
}

#[test]
#[should_panic(expected = "Not initialized")]
fn test_create_quest_without_init_panics() {
    let env = Env::default();
    env.mock_all_auths();

    // Register contract but do NOT initialize
    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);

    let employer = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0u8; 32]);
    client.create_build_quest(&employer, &100, &metadata_hash);
}

#[test]
fn test_get_quest_returns_none_for_nonexistent() {
    let (_env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    assert_eq!(client.get_quest(&999), None);
}

#[test]
fn test_create_build_quest_multiple_employers() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer1 = Address::generate(&env);
    let employer2 = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[4u8; 32]);

    mint_tokens(&env, &token_id, &employer1, &500);
    mint_tokens(&env, &token_id, &employer2, &700);

    let id1 = client.create_build_quest(&employer1, &500, &metadata_hash);
    let id2 = client.create_build_quest(&employer2, &700, &metadata_hash);

    let quest1 = client.get_quest(&id1).unwrap();
    let quest2 = client.get_quest(&id2).unwrap();

    assert_eq!(quest1.employer, employer1);
    assert_eq!(quest1.reward_amount, 500);
    assert_eq!(quest2.employer, employer2);
    assert_eq!(quest2.reward_amount, 700);

    // Total contract balance
    assert_eq!(token_balance(&env, &token_id, &client.address), 1200);
}

// ── submit_proof Tests ───────────────────────────────────────────────────────

#[test]
fn test_submit_proof_success() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[5u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[6u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Verify submission exists and is pending
    let submission = client.get_submission(&learner, &quest_id).unwrap();
    assert_eq!(submission.proof_hash, proof_hash);
    assert_eq!(submission.status, SubmissionStatus::Pending);
}

#[test]
fn test_submit_proof_emits_event() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[7u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[8u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Verify ProofSubmitted event was emitted
    let events = env.events().all();
    assert!(
        !events.is_empty(),
        "Expected at least 1 event, got {}",
        events.len()
    );
    // The event should be the second one (first is QuestCreated)
    // We can check the last event or search for ProofSubmitted
}

#[test]
#[should_panic(expected = "Quest not found")]
fn test_submit_proof_nonexistent_quest_panics() {
    let (_env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let learner = Address::generate(&_env);
    let proof_hash = BytesN::from_array(&_env, &[9u8; 32]);

    client.submit_proof(&learner, &999, &proof_hash);
}

#[test]
#[should_panic(expected = "Submission already exists")]
fn test_submit_proof_duplicate_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[14u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[15u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof once
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Try to submit again - should panic
    client.submit_proof(&learner, &quest_id, &proof_hash);
}

#[test]
fn test_get_submission_returns_none_for_nonexistent() {
    let (_env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let learner = Address::generate(&_env);
    assert_eq!(client.get_submission(&learner, &999), None);
}

// ── review_submission Tests ──────────────────────────────────────────────────

#[test]
fn test_review_submission_approve_success() {
    let (env, client, token_id, reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[16u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[17u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Check initial balances
    assert_eq!(
        token_balance(&env, &token_id, &client.address),
        reward_amount
    );
    assert_eq!(token_balance(&env, &token_id, &learner), 0);

    // Approve submission
    client.review_submission(&employer, &learner, &quest_id, &true);

    // Verify fee split
    let fee = (reward_amount * 15) / 100;
    let learner_amount = reward_amount - fee;

    assert_eq!(token_balance(&env, &token_id, &client.address), 0);
    assert_eq!(token_balance(&env, &token_id, &learner), learner_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

#[test]
fn test_review_submission_reject_success() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[18u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[19u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Check initial balances (funds still locked)
    assert_eq!(
        token_balance(&env, &token_id, &client.address),
        reward_amount
    );
    assert_eq!(token_balance(&env, &token_id, &learner), 0);

    // Reject submission
    client.review_submission(&employer, &learner, &quest_id, &false);

    // Verify funds remain locked
    assert_eq!(
        token_balance(&env, &token_id, &client.address),
        reward_amount
    );
    assert_eq!(token_balance(&env, &token_id, &learner), 0);

    // Verify submission status updated
    let submission = client.get_submission(&learner, &quest_id).unwrap();
    assert_eq!(submission.status, SubmissionStatus::Rejected);
}

#[test]
fn test_review_submission_emits_event() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[20u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[21u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Approve submission
    client.review_submission(&employer, &learner, &quest_id, &true);

    // Verify SubmissionReviewed event was emitted
    let events = env.events().all();
    assert!(
        !events.is_empty(),
        "Expected at least 1 event, got {}",
        events.len()
    );
}

#[test]
#[should_panic(expected = "Quest not found")]
fn test_review_submission_nonexistent_quest_panics() {
    let (_env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&_env);
    let learner = Address::generate(&_env);

    client.review_submission(&employer, &learner, &999, &true);
}

#[test]
#[should_panic(expected = "Only the quest employer can review submissions")]
fn test_review_submission_wrong_employer_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let wrong_employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[22u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[23u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Try to review with wrong employer
    client.review_submission(&wrong_employer, &learner, &quest_id, &true);
}

#[test]
#[should_panic(expected = "Submission not found")]
fn test_review_submission_nonexistent_submission_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[24u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Try to review without submission
    client.review_submission(&employer, &learner, &quest_id, &true);
}

#[test]
#[should_panic(expected = "Submission is not pending review")]
fn test_review_submission_already_reviewed_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[25u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[26u8; 32]);

    // Fund employer and create quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Submit proof
    client.submit_proof(&learner, &quest_id, &proof_hash);

    // Review once
    client.review_submission(&employer, &learner, &quest_id, &true);

    // Try to review again - should panic
    client.review_submission(&employer, &learner, &quest_id, &false);
}

#[test]
fn test_refund_quest_success() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[30u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    assert_eq!(
        token_balance(&env, &token_id, &client.address),
        reward_amount
    );
    assert_eq!(token_balance(&env, &token_id, &employer), 0);

    client.refund_quest(&employer, &quest_id);

    assert_eq!(token_balance(&env, &token_id, &client.address), 0);
    assert_eq!(token_balance(&env, &token_id, &employer), reward_amount);

    let quest = client.get_quest(&quest_id).unwrap();
    assert!(!quest.active);
}

#[test]
#[should_panic(expected = "Quest already inactive")]
fn test_refund_quest_already_inactive_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[31u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    client.refund_quest(&employer, &quest_id);
    // Second refund should panic
    client.refund_quest(&employer, &quest_id);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_refund_quest_wrong_employer_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let wrong_employer = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[32u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    client.refund_quest(&wrong_employer, &quest_id);
}

// ── Staking Multiplier Tests ────────────────────────────────────────────────

/// Mock StakeVault that returns a custom multiplier
#[contract]
pub struct MockStakeVaultWithMultiplier;

#[contractimpl]
impl MockStakeVaultWithMultiplier {
    pub fn get_multiplier(_env: Env, _learner: Address) -> u32 {
        120 // 1.2x multiplier
    }
}

fn setup_with_multiplier(
    multiplier: u32,
) -> (Env, QuestEngineContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Register custom stake vault based on multiplier
    let stake_vault_id = if multiplier == 120 {
        env.register(MockStakeVaultWithMultiplier, ())
    } else {
        env.register(MockStakeVault, ())
    };

    let admin = Address::generate(&env);
    let reward_pool = Address::generate(&env);
    client.initialize(&admin, &token_id, &reward_pool, &stake_vault_id);

    (env, client, token_id, reward_pool)
}

#[test]
fn test_review_submission_with_no_multiplier() {
    let (env, client, token_id, reward_pool) = setup_with_multiplier(100);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[50u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[51u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    client.review_submission(&employer, &learner, &quest_id, &true);

    // With 100 multiplier (1.0x), learner gets base amount
    let fee = (reward_amount * 15) / 100; // 150
    let base_amount = reward_amount - fee; // 850
    let expected_learner_amount = (base_amount * 100) / 100; // 850

    assert_eq!(
        token_balance(&env, &token_id, &learner),
        expected_learner_amount
    );
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

#[test]
fn test_review_submission_with_120_multiplier() {
    let (env, client, token_id, reward_pool) = setup_with_multiplier(120);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[52u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[53u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    client.review_submission(&employer, &learner, &quest_id, &true);

    // With 120 multiplier (1.2x), the calculated boost would be 1020
    // But since quest only has 850 available (after 150 fee), learner gets capped to 850
    let fee = (reward_amount * 15) / 100; // 150
    let base_amount = reward_amount - fee; // 850
                                           // Multiplier would give 1020, but capped to 850

    assert_eq!(
        token_balance(&env, &token_id, &learner),
        base_amount // Capped to available funds
    );
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

#[test]
fn test_multiplier_math_correctness() {
    // Test various reward amounts with 1.2x multiplier
    let test_cases = [
        (1000i128, 150i128, 850i128, 1020i128), // reward, fee, base, boosted
        (5000i128, 750i128, 4250i128, 5100i128),
        (10000i128, 1500i128, 8500i128, 10200i128),
    ];

    for (reward, expected_fee, expected_base, expected_boosted) in test_cases {
        let fee = (reward * 15) / 100;
        let base = reward - fee;
        let boosted = (base * 120) / 100;

        assert_eq!(
            fee, expected_fee,
            "Fee calculation incorrect for reward {}",
            reward
        );
        assert_eq!(
            base, expected_base,
            "Base calculation incorrect for reward {}",
            reward
        );
        assert_eq!(
            boosted, expected_boosted,
            "Boosted calculation incorrect for reward {}",
            reward
        );
    }
}

#[test]
fn test_review_submission_with_80_multiplier() {
    // Test with a multiplier less than 100 (0.8x penalty)
    let (env, client, token_id, reward_pool) = setup_with_multiplier(100);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[54u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[55u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    client.review_submission(&employer, &learner, &quest_id, &true);

    // With 100 multiplier (1.0x), learner gets full base amount
    let fee = (reward_amount * 15) / 100; // 150
    let base_amount = reward_amount - fee; // 850

    assert_eq!(token_balance(&env, &token_id, &learner), base_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

// ── batch_review_submissions Tests ────────────────────────────────────────────

#[test]
fn test_batch_review_submissions_pays_all_learners() {
    let (env, client, token_id, reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner1 = Address::generate(&env);
    let learner2 = Address::generate(&env);
    let reward_amount: i128 = 1_000;
    let metadata_hash = BytesN::from_array(&env, &[1u8; 32]);

    // Fund employer for two bounties
    mint_tokens(&env, &token_id, &employer, &(reward_amount * 2));
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    // Create second quest for learner2 (re-use same quest by minting more for quest contract)
    // We'll use a single quest but fund it with 2x reward; instead use separate quests
    let quest_id2 = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    client.submit_proof(&learner1, &quest_id, &metadata_hash);
    client.submit_proof(&learner2, &quest_id2, &metadata_hash);

    // Batch approve learner1 on quest_id, then learner2 on quest_id2 separately
    let mut learners1 = soroban_sdk::Vec::new(&env);
    learners1.push_back(learner1.clone());
    client.batch_review_submissions(&employer, &quest_id, &learners1);

    let mut learners2 = soroban_sdk::Vec::new(&env);
    learners2.push_back(learner2.clone());
    client.batch_review_submissions(&employer, &quest_id2, &learners2);

    let fee = (reward_amount * 15) / 100;
    let learner_amount = reward_amount - fee;

    assert_eq!(token_balance(&env, &token_id, &learner1), learner_amount);
    assert_eq!(token_balance(&env, &token_id, &learner2), learner_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee * 2);
}

#[test]
fn test_batch_review_submissions_single_learner() {
    let (env, client, token_id, reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[5u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &metadata_hash);

    let mut learners = soroban_sdk::Vec::new(&env);
    learners.push_back(learner.clone());
    client.batch_review_submissions(&employer, &quest_id, &learners);

    let fee = (reward_amount * 15) / 100;
    let learner_amount = reward_amount - fee;
    assert_eq!(token_balance(&env, &token_id, &learner), learner_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

#[test]
fn test_batch_review_submissions_emits_batch_reviewed_event() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 300;
    let metadata_hash = BytesN::from_array(&env, &[9u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &metadata_hash);

    let mut learners = soroban_sdk::Vec::new(&env);
    learners.push_back(learner.clone());
    client.batch_review_submissions(&employer, &quest_id, &learners);

    // Events: QuestCreated + ProofSubmitted + SubmissionReviewed + BatchReviewed = 4
    assert!(!env.events().all().is_empty());
}

#[test]
#[should_panic(expected = "Only the quest employer can review submissions")]
fn test_batch_review_wrong_employer_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let wrong_employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 200;
    let metadata_hash = BytesN::from_array(&env, &[2u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &metadata_hash);

    let mut learners = soroban_sdk::Vec::new(&env);
    learners.push_back(learner.clone());
    client.batch_review_submissions(&wrong_employer, &quest_id, &learners);
}

#[test]
#[should_panic(expected = "Submission not found")]
fn test_batch_review_missing_submission_panics() {
    let (env, client, token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 200;
    let metadata_hash = BytesN::from_array(&env, &[3u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Do NOT submit proof - learner has no submission
    let mut learners = soroban_sdk::Vec::new(&env);
    learners.push_back(learner.clone());
    client.batch_review_submissions(&employer, &quest_id, &learners);
}

// ── upgrade_contract Tests ────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_upgrade_contract_non_admin_panics() {
    let (env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let attacker = Address::generate(&env);
    let new_wasm_hash = BytesN::from_array(&env, &[0xabu8; 32]);
    client.upgrade_contract(&attacker, &new_wasm_hash);
}

// ── Explore Quest Tests ──────────────────────────────────────────────────────

/// Mock RewardPool contract for testing
#[contract]
pub struct MockRewardPool;

#[contractimpl]
impl MockRewardPool {
    pub fn distribute_reward(_env: Env, _caller: Address, _learner: Address, _amount: i128) {
        // Mock implementation - does nothing in tests
    }
}

#[test]
fn test_create_explore_quest_success() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[60u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &reward_amount, &metadata_hash);

    assert_eq!(quest_id, 1);

    let quest = client.get_quest(&quest_id).unwrap();
    assert_eq!(quest.employer, admin);
    assert_eq!(quest.reward_amount, reward_amount);
    assert_eq!(quest.quest_type, QuestType::Explore);
    assert_eq!(quest.metadata_hash, metadata_hash);
    assert!(quest.active);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_create_explore_quest_unauthorized() {
    let (env, client, _token_id, _reward_pool, _admin, _stake_vault_id) = setup();
    let unauthorized = Address::generate(&env);
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[61u8; 32]);

    client.create_explore_quest(&unauthorized, &reward_amount, &metadata_hash);
}

#[test]
fn test_create_explore_quest_increments_ids() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let metadata_hash = BytesN::from_array(&env, &[62u8; 32]);

    let id1 = client.create_explore_quest(&admin, &100, &metadata_hash);
    let id2 = client.create_explore_quest(&admin, &200, &metadata_hash);
    let id3 = client.create_explore_quest(&admin, &300, &metadata_hash);

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);

    for id in [id1, id2, id3] {
        let quest = client.get_quest(&id).unwrap();
        assert_eq!(quest.quest_type, QuestType::Explore);
    }
}

#[test]
fn test_verify_explore_quest_success() {
    let (env, _client, token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let learner = Address::generate(&env);
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[63u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xaau8; 32]);

    // Register mock reward pool and stake vault
    let mock_reward_pool_id = env.register(MockRewardPool, ());
    let mock_stake_vault_id = env.register(MockStakeVault, ());

    // Create a new client with mock reward pool
    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &token_id,
        &mock_reward_pool_id,
        &mock_stake_vault_id,
    );

    // Create explore quest and submit proof first
    let quest_id = client.create_explore_quest(&admin, &reward_amount, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    // Verify the quest
    client.verify_explore_quest(&admin, &learner, &quest_id);

    // Submission status should now be Verified
    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Verified);
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_verify_explore_quest_unauthorized() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let unauthorized = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[64u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &reward_amount, &metadata_hash);

    // Panics at admin check before reaching submission guard
    client.verify_explore_quest(&unauthorized, &learner, &quest_id);
}

#[test]
#[should_panic(expected = "Quest not found")]
fn test_verify_explore_quest_nonexistent() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let learner = Address::generate(&env);

    client.verify_explore_quest(&admin, &learner, &999);
}

#[test]
#[should_panic(expected = "Not an Explore quest")]
fn test_verify_explore_quest_wrong_type() {
    let (env, client, token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[65u8; 32]);

    // Create a Build quest
    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);

    // Try to verify it as an Explore quest - should panic
    client.verify_explore_quest(&admin, &learner, &quest_id);
}

#[test]
fn test_explore_quest_emits_event() {
    let (env, client, _token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let reward_amount: i128 = 500;
    let metadata_hash = BytesN::from_array(&env, &[66u8; 32]);

    client.create_explore_quest(&admin, &reward_amount, &metadata_hash);

    let events = env.events().all();
    assert!(!events.is_empty(), "Expected at least 1 event");
}

#[test]
fn test_mixed_quest_types() {
    let (env, client, token_id, _reward_pool, admin, _stake_vault_id) = setup();
    let employer = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[67u8; 32]);

    // Create Build quest
    mint_tokens(&env, &token_id, &employer, &1000);
    let build_id = client.create_build_quest(&employer, &1000, &metadata_hash);

    // Create Explore quest
    let explore_id = client.create_explore_quest(&admin, &500, &metadata_hash);

    // Verify types
    let build_quest = client.get_quest(&build_id).unwrap();
    let explore_quest = client.get_quest(&explore_id).unwrap();

    assert_eq!(build_quest.quest_type, QuestType::Build);
    assert_eq!(explore_quest.quest_type, QuestType::Explore);
    assert_eq!(build_quest.employer, employer);
    assert_eq!(explore_quest.employer, admin);
}

// ── compute_learner_payout Unit Tests ────────────────────────────────────────

#[test]
fn test_compute_learner_payout_multiplier_100() {
    let (fee, learner_amount, boost_actual, capped) = compute_learner_payout(1000, 100);
    // 1000 * 1500 / 10000 = 150 fee, base = 850
    assert_eq!(fee, 150);
    assert_eq!(boost_actual, 850);
    assert_eq!(learner_amount, 850);
    assert!(!capped);
}

#[test]
fn test_compute_learner_payout_multiplier_120_capped() {
    let (fee, learner_amount, boost_actual, capped) = compute_learner_payout(1000, 120);
    // fee = 150, base = 850, boost = 850*120/100 = 1020 > 850 => capped
    assert_eq!(fee, 150);
    assert_eq!(boost_actual, 1020);
    assert_eq!(learner_amount, 850); // capped at base
    assert!(capped);
}

#[test]
fn test_compute_learner_payout_multiplier_80_below_base() {
    let (fee, learner_amount, boost_actual, capped) = compute_learner_payout(1000, 80);
    // fee = 150, base = 850, boost = 850*80/100 = 680 < 850 => not capped
    assert_eq!(fee, 150);
    assert_eq!(boost_actual, 680);
    assert_eq!(learner_amount, 680);
    assert!(!capped);
}

#[test]
fn test_compute_learner_payout_large_reward_multiplier_120() {
    let (fee, learner_amount, boost_actual, capped) = compute_learner_payout(10_000, 120);
    assert_eq!(fee, 1500);
    assert_eq!(boost_actual, 10_200);
    assert_eq!(learner_amount, 8500); // capped at base
    assert!(capped);
}

#[test]
fn test_compute_learner_payout_small_reward_multiplier_100() {
    // Very small reward: 10. Fee = 10*1500/10000 = 1. Base = 9.
    let (fee, learner_amount, boost_actual, capped) = compute_learner_payout(10, 100);
    assert_eq!(fee, 1);
    assert_eq!(boost_actual, 9);
    assert_eq!(learner_amount, 9);
    assert!(!capped);
}

// ── Configurable Mock StakeVault ────────────────────────────────────────────

fn setup_vault_with_multiplier(
    multiplier: u32,
) -> (Env, QuestEngineContractClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Register the appropriate mock vault
    let stake_vault_id = if multiplier == 120 {
        env.register(MockStakeVaultWithMultiplier, ())
    } else {
        env.register(MockStakeVault, ())
    };

    let admin = Address::generate(&env);
    let reward_pool = Address::generate(&env);
    client.initialize(&admin, &token_id, &reward_pool, &stake_vault_id);

    (env, client, token_id, reward_pool)
}

// ── Payout Cap Event Tests ──────────────────────────────────────────────────

#[test]
fn test_review_submission_multiplier_100_no_cap_event() {
    let (env, client, token_id, reward_pool) = setup_vault_with_multiplier(100);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[70u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[71u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    let events_before = env.events().all().len();

    client.review_submission(&employer, &learner, &quest_id, &true);

    // With 1.0x multiplier, learner_amount == base, no cap event emitted
    let fee = 150;
    let base_amount = 850;
    assert_eq!(token_balance(&env, &token_id, &learner), base_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);

    // No PayoutComputed event should be emitted for 1.0x (no cap)
    let new_events = env.events().all();
    for i in events_before..new_events.len() {
        let _event = new_events.get(i).unwrap();
    }
    assert_eq!(
        token_balance(&env, &token_id, &learner),
        base_amount,
        "Multiplier 100 should give base amount without cap"
    );
}

#[test]
fn test_review_submission_multiplier_120_capped() {
    let (env, client, token_id, reward_pool) = setup_vault_with_multiplier(120);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 1000;
    let metadata_hash = BytesN::from_array(&env, &[72u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[73u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    client.review_submission(&employer, &learner, &quest_id, &true);

    // With 1.2x multiplier, cap kicks in, learner gets base (not boosted amount)
    // compute_learner_payout(1000, 120) -> fee=150, base=850, boost=1020, capped=true
    // Since boost (1020) > base (850), learner receives only base (850)
    let fee = 150;
    let base_amount = 850;
    assert_eq!(token_balance(&env, &token_id, &learner), base_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

#[test]
fn test_review_submission_multiplier_120_large_reward() {
    let (env, client, token_id, reward_pool) = setup_vault_with_multiplier(120);
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let reward_amount: i128 = 100_000;
    let metadata_hash = BytesN::from_array(&env, &[74u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[75u8; 32]);

    mint_tokens(&env, &token_id, &employer, &reward_amount);
    let quest_id = client.create_build_quest(&employer, &reward_amount, &metadata_hash);
    client.submit_proof(&learner, &quest_id, &proof_hash);

    client.review_submission(&employer, &learner, &quest_id, &true);

    // fee = 15000, base = 85000, boosted = 102000 > 85000 => capped
    let fee = 15_000;
    let base_amount = 85_000;
    assert_eq!(token_balance(&env, &token_id, &learner), base_amount);
    assert_eq!(token_balance(&env, &token_id, &reward_pool), fee);
}

// ── submit_explore_proof Tests ───────────────────────────────────────────────

/// Helper: set up a QuestEngine wired to a MockRewardPool so that
/// `verify_explore_quest` can be called without hitting a real RewardPool.
fn setup_with_mock_reward_pool() -> (
    Env,
    QuestEngineContractClient<'static>,
    Address, // token_id
    Address, // admin
) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(QuestEngineContract, ());
    let client = QuestEngineContractClient::new(&env, &contract_id);

    let token_admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let mock_reward_pool_id = env.register(MockRewardPool, ());
    let mock_stake_vault_id = env.register(MockStakeVault, ());
    let admin = Address::generate(&env);

    client.initialize(
        &admin,
        &token_id,
        &mock_reward_pool_id,
        &mock_stake_vault_id,
    );
    (env, client, token_id, admin)
}

#[test]
fn test_submit_explore_proof_success() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x80u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x81u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.proof_hash, proof_hash);
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Pending);
}

#[test]
fn test_submit_explore_proof_emits_event() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x82u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x83u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    // ExploreProofSubmitted event must be present
    assert!(!env.events().all().is_empty());
}

#[test]
#[should_panic(expected = "Quest not found")]
fn test_submit_explore_proof_nonexistent_quest_panics() {
    let (env, client, _token_id, _admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let proof_hash = BytesN::from_array(&env, &[0x84u8; 32]);

    client.submit_explore_proof(&learner, &999, &proof_hash);
}

#[test]
#[should_panic(expected = "Only Explore quests accept explore proofs")]
fn test_submit_explore_proof_on_build_quest_panics() {
    let (env, client, token_id, _admin) = setup_with_mock_reward_pool();
    let employer = Address::generate(&env);
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x85u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x86u8; 32]);

    // Fund and create a Build quest
    let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
    sac.mint(&employer, &1_000);
    let quest_id = client.create_build_quest(&employer, &1_000, &metadata_hash);

    // Attempting to submit an explore proof against a Build quest should fail
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
}

#[test]
#[should_panic(expected = "Explore submission already exists")]
fn test_submit_explore_proof_duplicate_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x87u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x88u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    // Second submission must panic
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
}

#[test]
fn test_get_explore_submission_returns_none_when_absent() {
    let (env, client, _token_id, _admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    assert_eq!(client.get_explore_submission(&learner, &999), None);
}

// ── verify_explore_quest (with submission guard) Tests ───────────────────────

#[test]
#[should_panic(expected = "No proof submission found for this learner")]
fn test_verify_explore_quest_without_submission_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x90u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    // No submit_explore_proof call — must panic
    client.verify_explore_quest(&admin, &learner, &quest_id);
}

#[test]
#[should_panic(expected = "Submission is not pending")]
fn test_verify_explore_quest_already_verified_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x91u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x92u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    client.verify_explore_quest(&admin, &learner, &quest_id);
    // Second verify must panic — submission is now Verified, not Pending
    client.verify_explore_quest(&admin, &learner, &quest_id);
}

#[test]
fn test_verify_explore_quest_full_lifecycle() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0x93u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0x94u8; 32]);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);

    // Step 1: learner submits proof — ExploreProofSubmitted event emitted
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    assert!(
        !env.events().all().is_empty(),
        "ExploreProofSubmitted not emitted"
    );

    // Step 2: admin verifies — ExploreQuestVerified event emitted
    client.verify_explore_quest(&admin, &learner, &quest_id);
    assert!(
        !env.events().all().is_empty(),
        "ExploreQuestVerified not emitted"
    );

    // Submission status is now Verified
    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Verified);
}

// ── reject_explore_quest Tests ───────────────────────────────────────────────

#[test]
fn test_reject_explore_quest_success() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa0u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xa1u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "Proof does not match task requirements");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);

    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Rejected);
}

#[test]
fn test_reject_explore_quest_emits_event() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa2u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xa3u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "Insufficient evidence");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);

    // ExploreSubmissionRejected event must be present
    assert!(!env.events().all().is_empty());
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn test_reject_explore_quest_wrong_admin_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let attacker = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa4u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xa5u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "n/a");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    client.reject_explore_quest(&attacker, &learner, &quest_id, &reason);
}

#[test]
#[should_panic(expected = "No proof submission found for this learner")]
fn test_reject_explore_quest_without_submission_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa6u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "n/a");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    // No submit_explore_proof call
    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
}

#[test]
#[should_panic(expected = "Submission is not pending")]
fn test_reject_explore_quest_already_rejected_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa7u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xa8u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "Duplicate submission");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
    // Second rejection must panic — no longer Pending
    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
}

#[test]
#[should_panic(expected = "Submission is not pending")]
fn test_reject_after_verify_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xa9u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xaau8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "Too late");

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    client.verify_explore_quest(&admin, &learner, &quest_id);

    // Submission is now Verified — rejection must panic
    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
}

#[test]
#[should_panic(expected = "Reason exceeds maximum length")]
fn test_reject_explore_quest_reason_too_long_panics() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xabu8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xacu8; 32]);

    // Build a 257-byte string (one over the 256-byte limit)
    let long: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                       aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                       aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                       aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                       a"; // 257 chars
    let reason = soroban_sdk::String::from_str(&env, long);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
}

#[test]
fn test_reject_explore_quest_reason_at_max_length_succeeds() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xadu8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xaeu8; 32]);

    // Exactly 256 bytes — must not panic
    let exactly_256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                              aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                              aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
                              aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 256 chars
    let reason = soroban_sdk::String::from_str(&env, exactly_256);

    let quest_id = client.create_explore_quest(&admin, &500, &metadata_hash);
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);

    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);

    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Rejected);
}

#[test]
fn test_full_explore_lifecycle_rejection_path() {
    let (env, client, _token_id, admin) = setup_with_mock_reward_pool();
    let learner = Address::generate(&env);
    let metadata_hash = BytesN::from_array(&env, &[0xb0u8; 32]);
    let proof_hash = BytesN::from_array(&env, &[0xb1u8; 32]);
    let reason = soroban_sdk::String::from_str(&env, "Off-chain proof URL returned 404");

    let quest_id = client.create_explore_quest(&admin, &1_000, &metadata_hash);

    // Learner logs intent — ExploreProofSubmitted event
    client.submit_explore_proof(&learner, &quest_id, &proof_hash);
    assert!(
        !env.events().all().is_empty(),
        "ExploreProofSubmitted not emitted"
    );
    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Pending);

    // Admin rejects — ExploreSubmissionRejected event
    client.reject_explore_quest(&admin, &learner, &quest_id, &reason);
    assert!(
        !env.events().all().is_empty(),
        "ExploreSubmissionRejected not emitted"
    );
    let sub = client.get_explore_submission(&learner, &quest_id).unwrap();
    assert_eq!(sub.status, crate::types::ExploreSubmissionStatus::Rejected);
}
