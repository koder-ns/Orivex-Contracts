#![no_std]

/// Current storage schema version for this contract.
/// Increment this constant when making breaking changes to stored structs or
/// DataKey variants, and add the corresponding migration step in `migrate()`.
///
/// Version history:
///   0 – pre-versioning baseline (no Version key in storage)
///   1 – initial versioned schema; StakeInfo struct unchanged from v0
pub const VERSION: u32 = 1;

pub const STAKE_TIER_HIGH_BPS: u32 = 200;

pub const STAKE_TIER_LOW_BPS: u32 = 120;

pub const STAKE_TIER_NONE_BPS: u32 = 100;
// Operational notes — multiplier calculation is a 3-tier
// lookup bound by `TIER_LOW_STAKE_BOUND` and
// `TIER_HIGH_STAKE_BOUND`. Re-staking resets the lock
// timestamp. Lock period is one week by default
// (`DEFAULT_LOCK_PERIOD_SECONDS`).

pub const TIER_HIGH_STAKE_BOUND: i128 = 500;

pub const TIER_LOW_STAKE_BOUND: i128 = 100;

pub const DEFAULT_LOCK_PERIOD_SECONDS: u64 = 604800;
// Crate overview — stake lock holding and multiplier computation.
// Provides `get_multiplier(user)` for cross-contract use by
// QuestEngine on review-time payout calculation.
use soroban_sdk::{contract, contractevent, contractimpl, token, Address, BytesN, Env};

pub mod types;
use types::{DataKey, StakeInfo};

#[contract]
pub struct StakeVault;

#[contractevent]
pub struct StakeVaultInitialized {
    #[topic]
    pub admin: Address,
    #[topic]
    pub token: Address,
}

#[contractevent]
pub struct Staked {
    #[topic]
    pub user: Address,
    pub amount: i128,
    pub total_staked: i128,
    pub lock_timestamp: u64,
}

#[contractevent]
pub struct Unstaked {
    #[topic]
    pub user: Address,
    pub amount: i128,
}

#[contractevent]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
}

#[contractimpl]
impl StakeVault {
    /// Initializes the StakeVault with admin and reward token
    /// addresses and emits `StakeVaultInitialized`. Admin-only at
    /// deploy time. Re-initialization panics with
    /// `"Already initialized"`.
    pub fn initialize(env: Env, admin: Address, token: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);

        StakeVaultInitialized { admin, token }.publish(&env);
    }

    /// Locks tokens for the configured lock period and resets the
    /// caller's `lock_timestamp` to the current ledger time. Multi-call
    /// stakes accumulate in the same `StakeInfo.amount` field.
    pub fn stake(env: Env, user: Address, amount: i128) {
        user.require_auth();

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let token_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token_id);

        token_client.transfer(&user, env.current_contract_address(), &amount);

        let now = env.ledger().timestamp();

        let mut stake_info: StakeInfo = env
            .storage()
            .persistent()
            .get(&DataKey::UserStake(user.clone()))
            .unwrap_or(StakeInfo {
                amount: 0,
                lock_timestamp: now,
            });

        stake_info.amount += amount;
        stake_info.lock_timestamp = now;

        env.storage()
            .persistent()
            .set(&DataKey::UserStake(user.clone()), &stake_info);

        Staked {
            user,
            amount,
            total_staked: stake_info.amount,
            lock_timestamp: stake_info.lock_timestamp,
        }
        .publish(&env);
    }

    /// Releases the caller's full staked balance once the lock period
    /// has elapsed. After successful withdrawal the storage slot is
    /// cleared — subsequent `unstake` calls panic with `"No stake found"`.
    pub fn unstake(env: Env, user: Address) {
        user.require_auth();

        let stake_info: StakeInfo = env
            .storage()
            .persistent()
            .get(&DataKey::UserStake(user.clone()))
            .expect("No stake found");

        let lock_period: u64 = DEFAULT_LOCK_PERIOD_SECONDS;
        if env.ledger().timestamp() < stake_info.lock_timestamp + lock_period {
            panic!("Lock period active");
        }

        let token_id: Address = env
            .storage()
            .instance()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token_id);

        token_client.transfer(
            &env.current_contract_address(),
            user.clone(),
            &stake_info.amount,
        );

        env.storage()
            .persistent()
            .remove(&DataKey::UserStake(user.clone()));

        Unstaked {
            user,
            amount: stake_info.amount,
        }
        .publish(&env);
    }

    /// Returns a basis-points multiplier based on the user's staked
    /// amount. The scheme uses three tiers: 100 (default, 1.0x),
    /// 120 (≥100 stake, 1.2x), and 200 (≥500 stake, 2.0x). Quest
    /// review paths consult this value to scale payouts.
    pub fn get_multiplier(env: Env, user: Address) -> u32 {
        let stake_info: StakeInfo = env
            .storage()
            .persistent()
            .get(&DataKey::UserStake(user))
            .unwrap_or(StakeInfo {
                amount: 0,
                lock_timestamp: 0,
            });

        if stake_info.amount >= 500 {
            200
        } else if stake_info.amount >= 100 {
            120
        } else {
            100
        }
    }

    /// Replaces the StakeVault WASM with the supplied hash on the
    /// Soroban host. Admin-only. Emits `ContractUpgraded` on
    /// successful deployment.
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
        if admin != stored_admin {
            panic!("Unauthorized");
        }

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
        if admin != stored_admin {
            panic!("Unauthorized");
        }

        let current_version: u32 = env.storage().instance().get(&DataKey::Version).unwrap_or(0);

        assert!(current_version < VERSION, "Already at current version");

        // ── v0 → v1 ──────────────────────────────────────────────────────────
        // StakeInfo struct is wire-compatible between v0 and v1.
        // A future migration adding fields to StakeInfo would iterate
        // StakerCount addresses and rewrite each UserStake(addr) here.
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
}

mod test;
