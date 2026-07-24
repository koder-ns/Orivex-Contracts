#![no_std]

/// Current storage schema version for this contract.
/// Increment this constant when making breaking changes to stored structs or
/// DataKey variants, and add the corresponding migration step in `migrate()`.
///
/// Version history:
///   0 – pre-versioning baseline (no Version key in storage)
///   1 – initial versioned schema; Quest and Submission structs unchanged from v0
pub const VERSION: u32 = 1;

pub const BUILD_QUEST_PREFIX: &str = "build";

pub const EXPLORE_QUEST_PREFIX: &str = "explore";
// Operational notes — review paths cross-call
// `StakeVault.get_multiplier` for payout scaling. Explore-quest
// payouts route via `RewardPool.distribute_reward` (which
// requires the QuestEngine to be whitelisted on the
// RewardPool).

pub const MAX_QUEST_REWARD: i128 = 1_000_000_000_000_000;

pub const PLATFORM_FEE_BASIS_POINTS: u32 = 1500;
pub const DISPUTE_WINDOW_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days in seconds
// Crate overview — Build and Explore quests. Build quests are
// employer-funded and reviewed per submission. Explore quests are
// admin-verified and rewarded out of the RewardPool.

pub mod types;
pub use types::QuestType;
use types::{DataKey, Quest, Submission, SubmissionStatus, Dispute};

use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, token, Address, BytesN, Env, Vec,
};

#[contractclient(name = "StakeVaultClient")]
pub trait StakeVaultInterface {
    fn get_multiplier(env: Env, learner: Address) -> u32;
}

#[contractclient(name = "RewardPoolClient")]
pub trait RewardPoolInterface {
    fn distribute_reward(env: Env, caller: Address, learner: Address, amount: i128);
}

#[contractclient(name = "GovernanceClient")]
pub trait GovernanceInterface {
    fn get_proposal(env: Env, proposal_id: u32) -> types::governance::Proposal;
}

pub mod types {
    pub mod governance {
        use soroban_sdk::{contracttype, Address, BytesN};

        #[contracttype]
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct Proposal {
            pub id: u32,
            pub proposer: Address,
            pub metadata_hash: BytesN<32>,
            pub votes_for: u32,
            pub votes_against: u32,
            pub end_time: u64,
            pub executed: bool,
        }
    }
}

#[contractevent]
pub struct QuestCreated {
    #[topic]
    pub employer: Address,
    #[topic]
    pub quest_id: u32,
    pub reward_amount: i128,
}

#[contractevent]
pub struct ProofSubmitted {
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    pub proof_hash: BytesN<32>,
}

#[contractevent]
pub struct SubmissionReviewed {
    #[topic]
    pub employer: Address,
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    pub approved: bool,
}

#[contractevent]
pub struct QuestRefunded {
    #[topic]
    pub employer: Address,
    #[topic]
    pub quest_id: u32,
    pub amount: i128,
}

#[contractevent]
pub struct BatchReviewed {
    #[topic]
    pub employer: Address,
    #[topic]
    pub quest_id: u32,
    pub approved_count: u32,
}

#[contractevent]
pub struct PayoutComputed {
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    pub fee: i128,
    pub learner_amount: i128,
    pub boost_actual: i128,
    pub boost_capped: bool,
}

#[contractevent]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
}

#[contractevent]
pub struct ExploreQuestVerified {
    #[topic]
    pub admin: Address,
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    pub amount: i128,
}

#[contractevent]
pub struct RewardPoolUpdated {
    #[topic]
    pub admin: Address,
    pub new_address: Address,
}

#[contractevent]
pub struct StakeVaultUpdated {
    #[topic]
    pub admin: Address,
    #[topic]
    pub new_address: Address,
}

#[contractevent]
pub struct DisputeOpened {
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    #[topic]
    pub dispute_id: u32,
    pub reason: BytesN<32>,
}

#[contractevent]
pub struct DisputeResolved {
    #[topic]
    pub dispute_id: u32,
    #[topic]
    pub learner: Address,
    #[topic]
    pub quest_id: u32,
    pub override_approve: bool,
}

#[contractevent]
pub struct GovernanceUpdated {
    #[topic]
    pub admin: Address,
    #[topic]
    pub new_address: Address,
}

#[contract]
pub struct QuestEngineContract;

/// Computes the fee split and staking boost for a quest payout.
///
/// Returns `(fee, learner_amount, boost_actual, boost_capped)` where:
/// - `fee`: platform fee (15% of `reward`).
/// - `learner_amount`: actual tokens transferred to the learner.
/// - `boost_actual`: the boosted amount before cap (`base * multiplier_bps / 100`).
/// - `boost_capped`: true when the boost was truncated to the available balance.
pub fn compute_learner_payout(reward: i128, multiplier_bps: u32) -> (i128, i128, i128, bool) {
    let fee = (reward * PLATFORM_FEE_BASIS_POINTS as i128) / 10_000;
    let base = reward - fee;
    let boost_actual = (base * multiplier_bps as i128) / 100;
    let capped = boost_actual > base;
    let learner_amount = if capped { base } else { boost_actual };
    (fee, learner_amount, boost_actual, capped)
}

#[contractimpl]
impl QuestEngineContract {
    /// Initializes the QuestEngine contract with the token address and admin.
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        reward_pool: Address,
        stake_vault: Address,
        governance: Option<Address>,
    ) {
        if env.storage().instance().has(&DataKey::Token) {
            panic!("Already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage()
            .instance()
            .set(&DataKey::RewardPool, &reward_pool);
        env.storage()
            .instance()
            .set(&DataKey::StakeVault, &stake_vault);
        if let Some(gov) = governance {
            env.storage().instance().set(&DataKey::Governance, &gov);
        }
        env.storage().instance().set(&DataKey::QuestCounter, &0u32);
        env.storage().instance().set(&DataKey::DisputeCounter, &0u32);
    }

    /// Toggles the pause state of the contract (emergency circuit breaker).
    ///
    /// # Arguments
    /// * `admin` - The admin address (must match stored admin)
    /// * `status` - The pause status (true = paused, false = unpaused)
    ///
    /// # Panics
    /// * If contract is not initialized
    /// * If admin does not match stored admin
    /// * If admin authentication fails
    /// Sets the `IsPaused` flag in instance storage as a circuit
    /// breaker. Admin-only. When true, `review_submission` and
    /// `batch_review_submissions` panic early with
    /// `"Contract is paused"`.
    pub fn set_pause(env: Env, admin: Address, status: bool) {
        // 1. Fetch 'Admin' address from Instance storage
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");

        // 2. Assert admin == stored_admin
        if admin != stored_admin {
            panic!("Unauthorized");
        }

        // 3. admin.require_auth()
        admin.require_auth();

        // 4. Store pause status in Instance storage
        env.storage().instance().set(&DataKey::IsPaused, &status);
    }

    /// Updates the RewardPool address used for explore-quest payouts.
    /// Admin-only. Emits `RewardPoolUpdated` with the new address.
    pub fn set_reward_pool_address(env: Env, admin: Address, new_address: Address) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::RewardPool, &new_address);

        RewardPoolUpdated { admin, new_address }.publish(&env);
    }

    /// Updates the StakeVault address used for multiplier lookups.
    /// Admin-only. Emits `StakeVaultUpdated` with the new address.
    pub fn set_stake_vault_address(env: Env, admin: Address, new_address: Address) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::StakeVault, &new_address);

        StakeVaultUpdated { admin, new_address }.publish(&env);
    }

    /// Updates the Governance contract address used for dispute resolution.
    /// Admin-only. Emits `GovernanceUpdated` with the new address.
    pub fn set_governance_address(env: Env, admin: Address, new_address: Address) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::Governance, &new_address);

        GovernanceUpdated { admin, new_address }.publish(&env);
    }

    /// Allows an employer to lock USDC directly in the QuestEngine contract.
    /// This acts as an isolated vault specifically for B2B bounties.
    /// Employer-funded quest that is funded out of the employer's
    /// balance at create time. The full `reward_amount` is locked in
    /// the QuestEngine contract; review actions later split it 85 / 15
    /// between learner and reward-pool.
    pub fn create_build_quest(
        env: Env,
        employer: Address,
        reward_amount: i128,
        metadata_hash: BytesN<32>,
    ) -> u32 {
        // 1. employer.require_auth()
        employer.require_auth();

        // 2. Fetch token_client for the USDC asset.
        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token_address);

        // 3. call token_client.transfer(employer, env.current_contract_address(), reward_amount).
        token_client.transfer(&employer, env.current_contract_address(), &reward_amount);

        // 4. Increment Quest ID counter.
        let mut quest_id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::QuestCounter)
            .unwrap_or(0);
        quest_id += 1;
        env.storage()
            .instance()
            .set(&DataKey::QuestCounter, &quest_id);

        // 5. Create Quest struct with QuestType::Build.
        let quest = Quest {
            employer: employer.clone(),
            reward_amount,
            quest_type: QuestType::Build,
            metadata_hash,
            active: true,
        };

        // 6. Save to Persistent storage.
        env.storage()
            .persistent()
            .set(&DataKey::Quest(quest_id), &quest);

        // 7. Emit QuestCreated event.
        QuestCreated {
            employer,
            quest_id,
            reward_amount,
        }
        .publish(&env);

        quest_id
    }

    /// Creates an Explore Quest that will be funded by the RewardPool.
    /// Explore Quests are for off-chain actions verified by the admin.
    ///
    /// # Arguments
    /// * `admin` - The admin address (must match stored admin)
    /// * `reward_amount` - The amount to be paid from RewardPool upon verification
    /// * `metadata_hash` - Hash of the quest metadata (description, requirements, etc.)
    ///
    /// # Returns
    /// The ID of the newly created quest
    ///
    /// # Panics
    /// * If admin authentication fails
    /// * If admin does not match stored admin
    /// * If contract is not initialized
    /// Admin-only creation of an Explore Quest that the RewardPool
    /// will fund on verification. The employer field is set to the
    /// admin so that downstream payout flows can route via the
    /// RewardPool's `distribute_reward` call.
    pub fn create_explore_quest(
        env: Env,
        admin: Address,
        reward_amount: i128,
        metadata_hash: BytesN<32>,
    ) -> u32 {
        // 1. admin.require_auth()
        admin.require_auth();

        // 2. Verify admin
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(admin == stored_admin, "Unauthorized");

        // 3. Increment Quest ID counter
        let mut quest_id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::QuestCounter)
            .unwrap_or(0);
        quest_id += 1;
        env.storage()
            .instance()
            .set(&DataKey::QuestCounter, &quest_id);

        // 4. Create Quest struct with QuestType::Explore
        let quest = Quest {
            employer: admin.clone(),
            reward_amount,
            quest_type: QuestType::Explore,
            metadata_hash,
            active: true,
        };

        // 5. Save to Persistent storage
        env.storage()
            .persistent()
            .set(&DataKey::Quest(quest_id), &quest);

        // 6. Emit QuestCreated event
        QuestCreated {
            employer: admin,
            quest_id,
            reward_amount,
        }
        .publish(&env);

        quest_id
    }

    /// Returns a quest by its ID.
    /// Reads a Quest struct from persistent storage by ID. Returns
    /// `None` when the ID has no record so callers can branch on
    /// presence rather than panic.
    pub fn get_quest(env: Env, quest_id: u32) -> Option<Quest> {
        env.storage().persistent().get(&DataKey::Quest(quest_id))
    }

    /// Allows a learner to submit proof for a build quest.
    /// Stores a learner's proof hash for the given build quest in
    /// `DataKey::Submission`. The associated quest must be active and
    /// of `QuestType::Build`. Re-submission for the same pair panics
    /// with `"Submission already exists"`.
    pub fn submit_proof(env: Env, learner: Address, quest_id: u32, proof_hash: BytesN<32>) {
        // 1. learner.require_auth()
        learner.require_auth();

        // 2. Retrieve Quest. Assert it is active and QuestType == Build.
        let quest: Quest = env
            .storage()
            .persistent()
            .get(&DataKey::Quest(quest_id))
            .expect("Quest not found");
        if !quest.active {
            panic!("Quest is not active");
        }
        if quest.quest_type != QuestType::Build {
            panic!("Only Build quests accept submissions");
        }

        // 3. Construct DataKey::Submission(learner, quest_id).
        let submission_key = DataKey::Submission(learner.clone(), quest_id);

        // 4. Assert a submission doesn't already exist.
        if env.storage().persistent().has(&submission_key) {
            panic!("Submission already exists");
        }

        // 5. Save struct { proof_hash, status: SubmissionStatus::Pending, reviewed_at: None } to storage.
        let submission = Submission {
            proof_hash: proof_hash.clone(),
            status: SubmissionStatus::Pending,
            reviewed_at: None,
        };
        env.storage().persistent().set(&submission_key, &submission);

        // 6. Emit ProofSubmitted event.
        ProofSubmitted {
            learner,
            quest_id,
            proof_hash,
        }
        .publish(&env);
    }

    /// Returns a submission by learner and quest ID.
    /// Reads a learner's Submission struct for a given quest.
    /// `None` indicates no submission has been recorded yet for the
    /// (learner, quest_id) pair.
    pub fn get_submission(env: Env, learner: Address, quest_id: u32) -> Option<Submission> {
        env.storage()
            .persistent()
            .get(&DataKey::Submission(learner, quest_id))
    }

    /// Allows an employer to review and approve/reject a learner's submission.
    /// Approves or rejects a single submission, applying the staking
    /// multiplier from the configured StakeVault. The boosted learner
    /// payout is capped at the available post-fee balance so that
    /// employer-funded quests can never go negative.
    pub fn review_submission(
        env: Env,
        employer: Address,
        learner: Address,
        quest_id: u32,
        approve: bool,
    ) {
        // 0. Check if contract is paused
        let is_paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::IsPaused)
            .unwrap_or(false);
        assert!(!is_paused, "Contract is paused");

        // 1. employer.require_auth()
        employer.require_auth();

        // 2. Retrieve Quest. Assert quest.employer == employer.
        let quest: Quest = env
            .storage()
            .persistent()
            .get(&DataKey::Quest(quest_id))
            .expect("Quest not found");
        if quest.employer != employer {
            panic!("Only the quest employer can review submissions");
        }

        // 3. Retrieve Submission. Assert status == Pending.
        let submission_key = DataKey::Submission(learner.clone(), quest_id);
        let mut submission: Submission = env
            .storage()
            .persistent()
            .get(&submission_key)
            .expect("Submission not found");
        if submission.status != SubmissionStatus::Pending {
            panic!("Submission is not pending review");
        }

        // 4. If approve == true:
        if approve {
            // a. Fetch token_client.transfer(env.current_contract_address(), learner, quest.reward_amount).
            let token_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .expect("Not initialized");
            let token_client = token::Client::new(&env, &token_address);

            // Fetch stake vault and get multiplier
            let stake_vault_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::StakeVault)
                .expect("Not initialized");
            let stake_vault_client = StakeVaultClient::new(&env, &stake_vault_address);
            let multiplier = stake_vault_client.get_multiplier(&learner);

            let (fee, learner_amount, boost_actual, boost_capped) =
                compute_learner_payout(quest.reward_amount, multiplier);

            let reward_pool: Address = env
                .storage()
                .instance()
                .get(&DataKey::RewardPool)
                .expect("Not initialized");

            token_client.transfer(&env.current_contract_address(), &reward_pool, &fee);
            token_client.transfer(&env.current_contract_address(), &learner, &learner_amount);

            if boost_capped {
                PayoutComputed {
                    learner: learner.clone(),
                    quest_id,
                    fee,
                    learner_amount,
                    boost_actual,
                    boost_capped,
                }
                .publish(&env);
            }

            submission.status = SubmissionStatus::Approved;
        } else {
            // 5. If approve == false:
            // a. Update submission status to Rejected.
            submission.status = SubmissionStatus::Rejected;
        }

        // 6. Set reviewed_at timestamp and save updated submission to Persistent storage.
        submission.reviewed_at = Some(env.ledger().timestamp());
        env.storage().persistent().set(&submission_key, &submission);

        // 7. Emit SubmissionReviewed event.
        SubmissionReviewed {
            employer,
            learner,
            quest_id,
            approved: approve,
        }
        .publish(&env);
    }

    /// Employer-only cancellation of an in-flight Build quest.
    /// Returns the locked `reward_amount` to the employer's wallet
    /// via the QuestEngine's token client and marks the quest
    /// inactive. Panics with `"Quest already inactive"` if the
    /// quest is already inactive.
    pub fn refund_quest(env: Env, employer: Address, quest_id: u32) {
        employer.require_auth();

        let mut quest: Quest = env
            .storage()
            .persistent()
            .get(&DataKey::Quest(quest_id))
            .expect("Quest not found");

        if quest.employer != employer {
            panic!("Unauthorized");
        }
        if !quest.active {
            panic!("Quest already inactive");
        }

        quest.active = false;
        env.storage()
            .persistent()
            .set(&DataKey::Quest(quest_id), &quest);

        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(
            &env.current_contract_address(),
            &employer,
            &quest.reward_amount,
        );

        QuestRefunded {
            employer,
            quest_id,
            amount: quest.reward_amount,
        }
        .publish(&env);
    }

    /// Approves multiple learner submissions in a single transaction.
    /// Executes the full fee-adjusted payout for each learner.
    /// Approves a vector of learner submissions against a single
    /// quest. Each submission must be `Pending`; the function
    /// panics on the first non-pending submission. Emits both
    /// individual `SubmissionReviewed` events and a single
    /// `BatchReviewed` summary event with the approved count.
    pub fn batch_review_submissions(
        env: Env,
        employer: Address,
        quest_id: u32,
        learners: Vec<Address>,
    ) {
        // 0. Check if contract is paused
        let is_paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::IsPaused)
            .unwrap_or(false);
        assert!(!is_paused, "Contract is paused");

        employer.require_auth();

        let quest: Quest = env
            .storage()
            .persistent()
            .get(&DataKey::Quest(quest_id))
            .expect("Quest not found");
        if quest.employer != employer {
            panic!("Only the quest employer can review submissions");
        }

        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token_address);

        let reward_pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::RewardPool)
            .expect("Not initialized");

        let mut approved_count: u32 = 0;
        for learner in learners.iter() {
            let submission_key = DataKey::Submission(learner.clone(), quest_id);
            let mut submission: Submission = env
                .storage()
                .persistent()
                .get(&submission_key)
                .expect("Submission not found");

            if submission.status != SubmissionStatus::Pending {
                panic!("Submission is not pending review");
            }

            let stake_vault_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::StakeVault)
                .expect("Not initialized");
            let stake_vault_client = StakeVaultClient::new(&env, &stake_vault_address);
            let multiplier = stake_vault_client.get_multiplier(&learner);

            let (fee, learner_amount, boost_actual, boost_capped) =
                compute_learner_payout(quest.reward_amount, multiplier);

            token_client.transfer(&env.current_contract_address(), &reward_pool, &fee);
            token_client.transfer(&env.current_contract_address(), &learner, &learner_amount);

            if boost_capped {
                PayoutComputed {
                    learner: learner.clone(),
                    quest_id,
                    fee,
                    learner_amount,
                    boost_actual,
                    boost_capped,
                }
                .publish(&env);
            }

            submission.status = SubmissionStatus::Approved;
            env.storage().persistent().set(&submission_key, &submission);

            SubmissionReviewed {
                employer: employer.clone(),
                learner,
                quest_id,
                approved: true,
            }
            .publish(&env);

            approved_count += 1;
        }

        BatchReviewed {
            employer,
            quest_id,
            approved_count,
        }
        .publish(&env);
    }

    /// Upgrades the contract WASM. Only callable by the Protocol Admin.
    ///
    /// After swapping the WASM, the caller **must** invoke `migrate()` in a
    /// subsequent transaction so that any storage-schema changes are applied
    /// before regular contract functions are used.
    pub fn upgrade_contract(env: Env, admin: Address, new_wasm_hash: BytesN<32>) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(admin == stored_admin, "Unauthorized");

        env.deployer()
            .update_current_contract_wasm(new_wasm_hash.clone());

        ContractUpgraded {
            admin,
            new_wasm_hash,
        }
        .publish(&env);
    }

    /// Applies any pending storage-schema migrations for the current WASM version.
    ///
    /// Must be called by the admin in the first transaction after `upgrade_contract`.
    ///
    /// # Version transition table
    /// | from | to | changes |
    /// |------|-----|---------|
    /// | 0    |  1  | Writes initial `Version = 1` marker; no struct changes |
    ///
    /// # Panics
    /// * If the caller is not the Protocol Admin.
    /// * If the on-chain version is already equal to or greater than `VERSION`.
    pub fn migrate(env: Env, admin: Address) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(admin == stored_admin, "Unauthorized");

        let current_version: u32 = env.storage().instance().get(&DataKey::Version).unwrap_or(0);

        assert!(current_version < VERSION, "Already at current version");

        // ── v0 → v1 ──────────────────────────────────────────────────────────
        // Quest and Submission structs are wire-compatible between v0 and v1.
        // A future migration adding fields would iterate QuestCount IDs and
        // rewrite each Quest(id) here.
        if current_version < 1 {
            // No data transformation required.
        }

        // ── write new version ─────────────────────────────────────────────────
        env.storage().instance().set(&DataKey::Version, &VERSION);
    }

    /// Returns the schema version currently stored in instance storage.
    /// Returns 0 when the contract was deployed before versioning was introduced.
    pub fn contract_version(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Version).unwrap_or(0)
    }

    /// Verifies an Explore Quest completion and triggers payout from RewardPool.
    /// Only the admin can call this function to reward off-chain actions.
    ///
    /// # Arguments
    /// * `admin` - The admin address (must match stored admin)
    /// * `learner` - The learner address to receive the reward
    /// * `quest_id` - The ID of the Explore Quest to verify
    ///
    /// # Panics
    /// * If admin authentication fails
    /// * If admin does not match stored admin
    /// * If quest is not found
    /// * If quest type is not Explore
    /// * If contract is not initialized
    /// Admin-only confirmation that a learner completed an off-chain
    /// action. Triggers a cross-contract `distribute_reward` call into
    /// the configured RewardPool. The QuestEngine must be whitelisted
    /// as an approved spender on RewardPool.
    pub fn verify_explore_quest(env: Env, admin: Address, learner: Address, quest_id: u32) {
        // 1. admin.require_auth()
        admin.require_auth();

        // 2. Verify admin
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        assert!(admin == stored_admin, "Unauthorized");

        // 3. Get quest
        let quest: Quest = env
            .storage()
            .persistent()
            .get(&DataKey::Quest(quest_id))
            .expect("Quest not found");

        // 4. Assert quest type is Explore
        assert!(
            quest.quest_type == QuestType::Explore,
            "Not an Explore quest"
        );

        // 5. Get reward pool address and create client
        let reward_pool_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::RewardPool)
            .expect("Not initialized");
        let reward_pool_client = RewardPoolClient::new(&env, &reward_pool_address);

        // 6. Distribute reward from RewardPool
        reward_pool_client.distribute_reward(
            &env.current_contract_address(),
            &learner,
            &quest.reward_amount,
        );

        // 7. Emit ExploreQuestVerified event
        ExploreQuestVerified {
            admin,
            learner,
            quest_id,
            amount: quest.reward_amount,
        }
        .publish(&env);
    }

    /// Allows a learner to open a dispute for a rejected submission within the dispute window.
    /// Learners can only dispute submissions that were rejected, and only within the
    /// DISPUTE_WINDOW_SECONDS (7 days) from when the submission was reviewed.
    pub fn dispute_submission(env: Env, learner: Address, quest_id: u32, reason: BytesN<32>) -> u32 {
        // 1. Require learner authentication
        learner.require_auth();

        // 2. Retrieve the submission
        let submission_key = DataKey::Submission(learner.clone(), quest_id);
        let submission: Submission = env
            .storage()
            .persistent()
            .get(&submission_key)
            .expect("Submission not found");

        // 3. Verify the submission was rejected
        if submission.status != SubmissionStatus::Rejected {
            panic!("Only rejected submissions can be disputed");
        }

        // 4. Check that the submission was reviewed and we're within the dispute window
        let reviewed_at = submission.reviewed_at.expect("Submission hasn't been reviewed yet");
        let current_time = env.ledger().timestamp();
        if current_time - reviewed_at > DISPUTE_WINDOW_SECONDS {
            panic!("Dispute window has expired - disputes must be opened within 7 days of review");
        }

        // We'll track the opened_at timestamp for the dispute
        // Increment dispute counter
        let mut dispute_id: u32 = env
            .storage()
            .instance()
            .get(&DataKey::DisputeCounter)
            .unwrap_or(0);
        dispute_id += 1;
        env.storage()
            .instance()
            .set(&DataKey::DisputeCounter, &dispute_id);

        // Create and store the dispute
        let dispute = Dispute {
            quest_id,
            learner: learner.clone(),
            reason: reason.clone(),
            opened_at: current_time,
            resolved: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Dispute(dispute_id), &dispute);

        // Emit DisputeOpened event
        DisputeOpened {
            learner,
            quest_id,
            dispute_id,
            reason,
        }
        .publish(&env);

        dispute_id
    }

    /// Admin-only function to resolve a dispute. Can override the original rejection to approve it.
    /// Emits a DisputeResolved event when completed.
    pub fn resolve_dispute(env: Env, admin: Address, dispute_id: u32, override_approve: bool) {
        // 1. Verify admin
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        admin.require_auth();

        // 2. Retrieve the dispute
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(dispute_id))
            .expect("Dispute not found");

        // 3. Verify dispute is not already resolved
        if dispute.resolved {
            panic!("Dispute has already been resolved");
        }

        // 4. Mark dispute as resolved
        dispute.resolved = true;
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(dispute_id), &dispute);

        // 5. If we need to override and approve, process the payout
        if override_approve {
            let quest: Quest = env
                .storage()
                .persistent()
                .get(&DataKey::Quest(dispute.quest_id))
                .expect("Quest not found");

            // Update submission status to Approved
            let submission_key = DataKey::Submission(dispute.learner.clone(), dispute.quest_id);
            let mut submission: Submission = env
                .storage()
                .persistent()
                .get(&submission_key)
                .expect("Submission not found");
            submission.status = SubmissionStatus::Approved;
            env.storage().persistent().set(&submission_key, &submission);

            // Process payout just like in review_submission
            let token_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .expect("Not initialized");
            let token_client = token::Client::new(&env, &token_address);

            // Fetch stake vault and get multiplier
            let stake_vault_address: Address = env
                .storage()
                .instance()
                .get(&DataKey::StakeVault)
                .expect("Not initialized");
            let stake_vault_client = StakeVaultClient::new(&env, &stake_vault_address);
            let multiplier = stake_vault_client.get_multiplier(&dispute.learner);

            let (fee, learner_amount, boost_actual, boost_capped) =
                compute_learner_payout(quest.reward_amount, multiplier);

            let reward_pool: Address = env
                .storage()
                .instance()
                .get(&DataKey::RewardPool)
                .expect("Not initialized");

            token_client.transfer(&env.current_contract_address(), &reward_pool, &fee);
            token_client.transfer(&env.current_contract_address(), &dispute.learner, &learner_amount);

            if boost_capped {
                PayoutComputed {
                    learner: dispute.learner.clone(),
                    quest_id: dispute.quest_id,
                    fee,
                    learner_amount,
                    boost_actual,
                    boost_capped,
                }
                .publish(&env);
            }
        }

        // 6. Emit DisputeResolved event
        DisputeResolved {
            dispute_id,
            learner: dispute.learner,
            quest_id: dispute.quest_id,
            override_approve,
        }
        .publish(&env);
    }

    /// Governance-only function to resolve a dispute based on voting results.
    /// Resolves the dispute if votes_for > votes_against, otherwise leaves it as rejected.
    pub fn resolve_dispute_via_governance(env: Env, dispute_id: u32, proposal_id: u32) {
        // 1. Get governance address
        let governance_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::Governance)
            .expect("Governance contract not configured");

        // 2. Create governance client and get the proposal
        let governance_client = GovernanceClient::new(&env, &governance_address);
        let proposal = governance_client.get_proposal(proposal_id);

        // 3. Verify proposal is executed and votes are in favor
        if !proposal.executed {
            panic!("Proposal has not been executed");
        }

        if proposal.votes_for <= proposal.votes_against {
            panic!("Insufficient votes to approve the dispute");
        }

        // 4. If proposal passes, resolve the dispute with override_approve = true
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        Self::resolve_dispute(env, stored_admin, dispute_id, true);
    }

    /// Returns a dispute by its ID.
    pub fn get_dispute(env: Env, dispute_id: u32) -> Option<Dispute> {
        env.storage().persistent().get(&DataKey::Dispute(dispute_id))
    }
}

#[cfg(test)]
mod test;