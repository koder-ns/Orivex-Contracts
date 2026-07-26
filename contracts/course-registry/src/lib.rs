#![no_std]

use contracts_common::errors::ContractError;

/// Schema version stored in instance storage.
///   0 – pre-versioning baseline (no Version key in storage)
///   1 – initial versioned schema; Course struct unchanged from v0
pub const VERSION: u32 = 1;

pub const INITIAL_COURSE_ID: u32 = 1;

pub const MAX_COURSE_ID: u32 = u32::MAX;
// Operational notes — storage costs are amortised over a
// single `Course` struct per ID and a separate progress
// record per (learner, course). Roster growth is bounded by
// Soroban's `u32` ID space and the 7-byte constraint for
// metadata references.

pub const DEFAULT_TOTAL_MODULES_BOUND: u32 = 1000;

pub const BASE_REWARD_AMOUNT: i128 = 10_0000000;
// Crate overview — manages the lifecycle of on-chain courses,
// their progress records, course completion mint of soulbound
// badges, and RewardPool payout triggering.
use soroban_sdk::{contract, contractevent, contractimpl, Address, BytesN, Env};

pub mod types;
use types::{Course, DataKey};

use badge_nft::BadgeNFTClient;
use reward_pool::RewardPoolClient;

#[contract]
pub struct CourseRegistry;

#[contractevent]
pub struct MetadataUpdated {
    #[topic]
    pub id: u32,
    #[topic]
    pub instructor: Address,
    pub new_hash: BytesN<32>,
}

#[contractevent]
pub struct CourseCreated {
    #[topic]
    pub id: u32,
    #[topic]
    pub instructor: Address,
    pub total_modules: u32,
}

#[contractevent]
pub struct CourseStatusChanged {
    #[topic]
    pub id: u32,
    pub active: bool,
}

#[contractevent]
pub struct OwnershipTransferred {
    #[topic]
    pub course_id: u32,
    #[topic]
    pub previous_instructor: Address,
    pub new_instructor: Address,
}

#[contractevent]
pub struct ModuleCompleted {
    #[topic]
    pub learner: Address,
    #[topic]
    pub course_id: u32,
    pub new_progress: u32,
}

#[contractevent]
pub struct CourseCompleted {
    #[topic]
    pub learner: Address,
    #[topic]
    pub course_id: u32,
    pub reward_amount: i128,
}

#[contractevent]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
}

#[contractimpl]
impl CourseRegistry {
    /// Sets the official Protocol Admin. Must be called once upon deployment.
    /// Sets the single Protocol Admin in instance storage at deploy time.
    /// Idempotent guards prevent re-initialization: the function panics if
    /// `DataKey::Admin` is already present. The caller must authenticate
    /// (`admin.require_auth()`) to bind the transaction signature to the
    /// supplied address, preventing front-runner attacks.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("{}", ContractError::AlreadyInitialized.msg());
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Registers the RewardPool contract address so the registry can trigger payouts on completion.
    /// Only callable by the Protocol Admin.
    /// Wires the RewardPool contract address used by `complete_module`.
    /// Only the Protocol Admin may call this; otherwise the call panics
    /// with `"Unauthorized: Caller is not the protocol admin"`. The
    /// RewardPool must additionally whitelist the CourseRegistry via
    /// `add_approved_spender` before payouts will execute.
    pub fn set_reward_pool_address(env: Env, admin: Address, reward_pool_address: Address) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(
            admin == stored_admin,
            "Unauthorized: Caller is not the protocol admin"
        );

        env.storage()
            .instance()
            .set(&DataKey::RewardPoolAddress, &reward_pool_address);
    }

    /// Registers the BadgeNFT contract address so the registry can mint badges on completion.
    /// Only callable by the Protocol Admin.
    /// Wires the BadgeNFT contract address so completed courses can mint
    /// soulbound badges for learners. Admin-only — fails with
    /// `"Unauthorized: Caller is not the protocol admin"` if the caller
    /// isn't the configured Protocol Admin.
    pub fn set_badge_nft_address(env: Env, admin: Address, badge_nft_address: Address) {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(
            admin == stored_admin,
            "Unauthorized: Caller is not the protocol admin"
        );

        env.storage()
            .instance()
            .set(&DataKey::BadgeNftAddress, &badge_nft_address);
    }

    /// Registers a new course on-chain.
    /// Allocates the next monotonically-increasing course ID and stores
    /// the resulting `Course` struct in persistent storage under
    /// `DataKey::Course(id)`. `total_modules` must be strictly positive;
    /// the cap on courses is bounded by the `u32` return value.
    pub fn create_course(
        env: Env,
        admin: Address,
        instructor: Address,
        total_modules: u32,
        metadata_hash: BytesN<32>,
    ) -> u32 {
        admin.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(
            admin == stored_admin,
            "Unauthorized: Caller is not the protocol admin"
        );

        assert!(total_modules > 0, "total_modules must be greater than 0");

        let current_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CourseCount)
            .unwrap_or(0);
        assert!(
            total_modules <= DEFAULT_TOTAL_MODULES_BOUND,
            "total_modules exceeds bound"
        );
        assert!(current_count < MAX_COURSE_ID, "Course ID limit reached");
        let new_id = current_count + INITIAL_COURSE_ID;
        env.storage().instance().set(&DataKey::CourseCount, &new_id);

        let course = Course {
            instructor: instructor.clone(),
            total_modules,
            metadata_hash,
            active: true,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Course(new_id), &course);

        CourseCreated {
            id: new_id,
            instructor,
            total_modules,
        }
        .publish(&env);

        new_id
    }

    /// Updates the IPFS metadata hash for a course. Only callable by the course instructor.
    /// Replaces a course's IPFS metadata hash with the supplied value.
    /// Only the current instructor is permitted to update; the function
    /// uses `course.instructor.require_auth()` for that check. The new
    /// hash must be a 32-byte BytesN pointing at IPFS CID metadata.
    pub fn update_metadata(env: Env, id: u32, new_hash: BytesN<32>) {
        let mut course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(id))
            .expect("Course not found");

        course.instructor.require_auth();

        let instructor = course.instructor.clone();
        course.metadata_hash = new_hash.clone();

        env.storage()
            .persistent()
            .set(&DataKey::Course(id), &course);

        MetadataUpdated {
            id,
            instructor,
            new_hash,
        }
        .publish(&env);
    }

    /// Enrolls a learner in an active course, initializing their progress to 0.
    /// Initializes a learner progress record at zero for the requested
    /// course. The first enrollment writes `0u32` to the
    /// `DataKey::Progress(learner, id)` slot. Panics with
    /// `"Learner already enrolled"` if a record already exists.
    pub fn enroll(env: Env, learner: Address, id: u32) {
        learner.require_auth();

        let course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(id))
            .expect("Course not found");

        assert!(course.active, "Course is not active");

        let progress_key = DataKey::Progress(learner.clone(), id);
        assert!(
            !env.storage().persistent().has(&progress_key),
            "Learner already enrolled"
        );

        env.storage().persistent().set(&progress_key, &0u32);
    }

    /// Helper to check the current total number of courses.
    /// Returns the total number of courses currently registered
    /// on-chain. Reads from `DataKey::CourseCount` instance storage
    /// and defaults to 0 if absent (e.g. before the first
    /// `create_course` call).
    pub fn course_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CourseCount)
            .unwrap_or(0)
    }

    /// Toggles a course's active status. Only callable by the Protocol Admin.
    /// Toggles the active flag on the target course and emits
    /// `CourseStatusChanged { id, active }`. Admin-only. The status
    /// change is persisted in `DataKey::Course(id)` and the course
    /// remains in storage so prior learner progress is preserved.
    pub fn set_course_status(env: Env, admin: Address, id: u32, active: bool) {
        // 1. Authenticate the admin cryptographically
        admin.require_auth();

        // 2. Verify caller is the officially registered Protocol Admin
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Contract not initialized");
        assert!(
            admin == stored_admin,
            "Unauthorized: Caller is not the protocol admin"
        );

        // 3. Retrieve the course using the CORRECT DataKey
        let mut course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(id))
            .expect("Course not found");

        // 4. Update the active status and save it
        course.active = active;
        env.storage()
            .persistent()
            .set(&DataKey::Course(id), &course);

        // 5. Emit the standard event
        CourseStatusChanged { id, active }.publish(&env);
    }

    /// Returns true if the learner has completed all modules in the course.
    /// Returns true when the learner's stored progress is at least
    /// `course.total_modules`. The check is defensive — progress
    /// values exceeding total_modules also count as finished.
    pub fn is_course_finished(env: Env, learner: Address, id: u32) -> bool {
        let course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(id))
            .expect("Course not found");

        let progress: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::Progress(learner, id))
            .unwrap_or(0);

        progress >= course.total_modules
    }

    /// Returns the full details of a specific course.
    ///
    /// # Arguments
    /// * `env` - The Soroban environment
    /// * `id` - The course ID
    ///
    /// # Returns
    /// The Course struct if found
    ///
    /// # Panics
    /// Panics if the course ID is invalid (course doesn't exist in storage)
    /// Reads a Course struct from persistent storage by ID. The
    /// function panics with `"Course not found"` when the ID has
    /// no record, which is the deliberate failure mode for an
    /// out-of-bounds lookup.
    pub fn get_course(env: Env, id: u32) -> Course {
        // 1. Construct DataKey::Course(id)
        let key = DataKey::Course(id);

        // 2. Fetch Course struct from Persistent storage
        // 3. Assert course exists (panic if not found)
        env.storage()
            .persistent()
            .get(&key)
            .expect("Course not found")
    }

    /// Returns a learner's completed module count for a course. Returns 0 if the learner has not enrolled.
    /// Reads a learner's current module-completion count for a
    /// course. Returns 0 when the learner has not enrolled (no
    /// matching `DataKey::Progress` slot), avoiding the need to
    /// explicitly call `enroll`.
    pub fn get_progress(env: Env, learner: Address, id: u32) -> u32 {
        let key = DataKey::Progress(learner, id);
        env.storage().persistent().get(&key).unwrap_or(0)
    }

    /// Transfers ownership of a course to a new instructor address.
    /// Only callable by the current instructor of the course.
    pub fn transfer_ownership(
        env: Env,
        current_instructor: Address,
        new_instructor: Address,
        course_id: u32,
    ) {
        let mut course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(course_id))
            .expect("Course not found");

        assert!(
            course.instructor == current_instructor,
            "Unauthorized: Caller is not the course instructor"
        );

        current_instructor.require_auth();

        course.instructor = new_instructor.clone();
        env.storage()
            .persistent()
            .set(&DataKey::Course(course_id), &course);

        OwnershipTransferred {
            course_id,
            previous_instructor: current_instructor,
            new_instructor,
        }
        .publish(&env);
    }

    /// Records a learner's completion of a module after off-chain quiz validation.
    /// Only callable by the authorized verifier (protocol admin).
    /// Records a verifier-confirmed module completion and emits the
    /// `ModuleCompleted` event. On the final module, this function
    /// additionally mints the soulbound badge and records a pending reward.
    ///
    /// Follows the checks-effects-interactions pattern: all state mutations
    /// happen before cross-contract calls. If the reward payout fails, the
    /// learner can retry via `claim_completion_reward`.
    pub fn complete_module(env: Env, verifier: Address, learner: Address, id: u32) {
        // ── CHECKS ──────────────────────────────────────────────────────
        verifier.require_auth();

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .unwrap_or_else(|| panic!("{}", ContractError::NotInitialized.msg()));
        assert!(
            verifier == stored_admin,
            "Unauthorized: Caller is not the protocol admin"
        );

        let course: Course = env
            .storage()
            .persistent()
            .get(&DataKey::Course(id))
            .expect("Course not found");

        let current_progress: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::Progress(learner.clone(), id))
            .unwrap_or(0);

        assert!(
            current_progress < course.total_modules,
            "Course already completed"
        );

        // ── EFFECTS ─────────────────────────────────────────────────────
        let new_progress = current_progress + 1;

        env.storage()
            .persistent()
            .set(&DataKey::Progress(learner.clone(), id), &new_progress);

        let course_completed = new_progress == course.total_modules;

        // Record pending reward if course is completed and RewardPool is configured
        if course_completed && env.storage().instance().has(&DataKey::RewardPoolAddress) {
            env.storage()
                .persistent()
                .set(&DataKey::PendingReward(learner.clone(), id), &true);
        }

        // ── INTERACTIONS ────────────────────────────────────────────────
        ModuleCompleted {
            learner: learner.clone(),
            course_id: id,
            new_progress,
        }
        .publish(&env);

        if course_completed {
            // Mint soulbound badge
            if let Some(badge_nft_address) = env
                .storage()
                .instance()
                .get::<DataKey, Address>(&DataKey::BadgeNftAddress)
            {
                let badge_nft = BadgeNFTClient::new(&env, &badge_nft_address);
                badge_nft.mint_badge(&env.current_contract_address(), &learner, &id);
            }

            // Attempt reward distribution; if it succeeds, clear the pending reward.
            // If it fails, the pending reward record allows the learner to retry.
            if Self::try_distribute_reward(&env, &learner, id) {
                env.storage()
                    .persistent()
                    .remove(&DataKey::PendingReward(learner.clone(), id));
            }
        }
    }

    /// Allows a learner to claim a pending course completion reward that
    /// was not distributed during `complete_module` (e.g. due to a
    /// transient RewardPool failure).
    ///
    /// Only succeeds if:
    /// - The learner has a pending reward record
    /// - The reward has not already been claimed
    pub fn claim_completion_reward(env: Env, learner: Address, course_id: u32) {
        learner.require_auth();

        let pending_key = DataKey::PendingReward(learner.clone(), course_id);
        assert!(
            env.storage().persistent().has(&pending_key),
            "No pending reward for this learner and course"
        );

        // Remove the pending reward record to prevent double-claiming
        env.storage().persistent().remove(&pending_key);

        // Distribute the reward
        Self::try_distribute_reward(&env, &learner, course_id);
    }

    /// Attempts to distribute the course completion reward. Returns true if
    /// the reward was successfully distributed, false if the RewardPool is
    /// not configured or the transfer failed.
    ///
    /// This is a best-effort attempt — it does not panic on failure, allowing
    /// the caller to handle the failure gracefully (e.g. by recording a
    /// pending reward).
    fn try_distribute_reward(env: &Env, learner: &Address, course_id: u32) -> bool {
        let reward_pool_address: Address = match env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RewardPoolAddress)
        {
            Some(addr) => addr,
            None => return false,
        };

        let reward_pool = RewardPoolClient::new(env, &reward_pool_address);
        let base_reward: i128 = BASE_REWARD_AMOUNT;

        match reward_pool.try_distribute_reward(
            &env.current_contract_address(),
            learner,
            &base_reward,
        ) {
            Ok(_) => {
                CourseCompleted {
                    learner: learner.clone(),
                    course_id,
                    reward_amount: base_reward,
                }
                .publish(env);
                true
            }
            Err(_) => false,
        }
    }

    /// Upgrades the contract WASM. Only callable by the Protocol Admin.
    /// Replaces the contract WASM with the supplied hash on the
    /// Soroban host. Admin-only — non-admins panic with
    /// `"Unauthorized"`. The `ContractUpgraded` event is published
    /// with both the admin's address and the new WASM hash.
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
    /// Reads the version stored on-chain, executes any required migration steps in
    /// order, and finally writes the new `VERSION` constant to instance storage.
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
        // Course struct is wire-compatible between v0 and v1. A future migration
        // that adds a field would iterate DataKey::CourseCount and rewrite each
        // Course(id) here.
        if current_version < 1 {
            // No data transformation required for this transition.
        }

        // ── write new version ─────────────────────────────────────────────────
        env.storage().instance().set(&DataKey::Version, &VERSION);
    }

    /// Returns the schema version currently stored in instance storage.
    /// Returns 0 when the contract was deployed before versioning was introduced.
    pub fn contract_version(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Version).unwrap_or(0)
    }
}

mod test;
