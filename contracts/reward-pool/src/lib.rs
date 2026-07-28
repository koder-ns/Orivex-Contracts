#![no_std]

/// Current storage schema version for this contract.
/// Increment this constant when making breaking changes to stored structs or
/// DataKey variants, and add the corresponding migration step in `migrate()`.
///
/// Version history:
///   0 – pre-versioning baseline (no Version key in storage)
///   1 – initial versioned schema; no struct changes from v0
pub const VERSION: u32 = 1;

pub const REWARD_TOKEN_DECIMALS: u32 = 7;

pub const MAX_SPENDERS: u32 = 256;
// Operational notes — the `IsPaused` flag is a global switch
// for `distribute_reward`. Fund recovery always routes via
// `emergency_sweep`, never via direct token transfer. Spender
// list entries are stored in persistent storage and persist
// across upgrades.

pub const MIN_PAYOUT_AMOUNT: i128 = 1;

pub const PLATFORM_FEE_BASIS_POINTS: u32 = 1500;
// Crate overview — central USDC reward distribution. Holds the
// reward-token balance and gates payouts behind an approved-
// spender allowlist.
use soroban_sdk::{contractclient, contractevent, Address, BytesN, Env};

pub mod types;

#[contractclient(name = "RewardPoolClient")]
pub trait RewardPoolInterface {
    fn initialize(env: Env, admin: Address, token: Address);
    fn add_approved_spender(env: Env, admin: Address, spender: Address);
    fn remove_approved_spender(env: Env, admin: Address, spender: Address);
    fn set_pause(env: Env, admin: Address, status: bool);
    fn distribute_reward(env: Env, caller: Address, learner: Address, amount: i128);
    fn fund_pool(env: Env, donor: Address, amount: i128);
    fn emergency_sweep(env: Env, admin: Address, recovery_wallet: Address);
    fn upgrade_contract(env: Env, admin: Address, new_wasm_hash: BytesN<32>);
    fn estimated_storage_footprint(env: Env) -> u32;
    fn migrate(env: Env, admin: Address);
    fn contract_version(env: Env) -> u32;
}

#[contractevent]
pub struct PoolInitialized {
    #[topic]
    pub admin: Address,
    #[topic]
    pub token: Address,
}

#[contractevent]
pub struct SpenderAdded {
    #[topic]
    pub spender: Address,
}

#[contractevent]
pub struct SpenderRemoved {
    #[topic]
    pub spender: Address,
}

#[contractevent]
pub struct RewardDistributed {
    #[topic]
    pub caller: Address,
    #[topic]
    pub learner: Address,
    pub amount: i128,
}

#[contractevent]
pub struct PoolFunded {
    #[topic]
    pub donor: Address,
    pub amount: i128,
}

#[contractevent]
pub struct EmergencySweep {
    #[topic]
    pub admin: Address,
    #[topic]
    pub recovery_wallet: Address,
    pub amount: i128,
}

#[contractevent]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
}

#[cfg(feature = "contract")]
mod contract_impl {
    use soroban_sdk::{contract, contractimpl, token, Address, BytesN, Env};

    use crate::types::DataKey;
    use crate::{
        ContractUpgraded, EmergencySweep, PoolFunded, PoolInitialized, RewardDistributed,
        SpenderAdded, SpenderRemoved,
    };

    #[contract]
    pub struct RewardPool;

    #[contractimpl]
    impl RewardPool {
        /// Initializes the RewardPool contract with admin and token addresses.
        ///
        /// # Arguments
        /// * `admin` - The admin address that will have administrative control
        /// * `token` - The SAC token address to be used as reward token
        ///
        /// # Panics
        /// * If contract is already initialized
        /// * If admin authentication fails
        /// Stores admin and reward-token addresses in instance storage and
        /// emits the `PoolInitialized` event. Both addresses are recorded
        /// on the first call; subsequent calls panic with
        /// `"Already initialized"`.
        pub fn initialize(env: Env, admin: Address, token: Address) {
            // 1. Check if already initialized
            if env.storage().instance().has(&DataKey::Admin) {
                panic!("Already initialized");
            }

            // 2. Require admin authentication
            admin.require_auth();

            // 3. Store admin in Instance storage
            env.storage().instance().set(&DataKey::Admin, &admin);

            // 4. Store token in Instance storage
            env.storage().instance().set(&DataKey::Token, &token);

            // 5. Emit PoolInitialized event
            PoolInitialized { admin, token }.publish(&env);
        }

        /// Adds a contract address to the approved spender whitelist.
        ///
        /// # Arguments
        /// * `admin` - The admin address (must match stored admin)
        /// * `spender` - The contract address to whitelist
        ///
        /// # Panics
        /// * If contract is not initialized
        /// * If admin does not match stored admin
        /// * If admin authentication fails
        /// Whitelist a caller contract so future `distribute_reward`
        /// calls from that contract's address are authorised. The
        /// spender is recorded under `DataKey::Spender(address)` in
        /// persistent storage. Re-whitelisting is allowed (idempotent).
        pub fn add_approved_spender(env: Env, admin: Address, spender: Address) {
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

            // 4. Save `true` to Persistent storage using DataKey::Spender(spender.clone())
            let spender_key = DataKey::Spender(spender.clone());
            let is_new: bool = !env.storage().persistent().has(&spender_key);
            env.storage().persistent().set(&spender_key, &true);

            // Increment the footprint counter only for genuinely new spenders.
            if is_new {
                let prev: u32 = env
                    .storage()
                    .instance()
                    .get(&DataKey::SpenderCount)
                    .unwrap_or(0);
                env.storage()
                    .instance()
                    .set(&DataKey::SpenderCount, &(prev + 1));
            }

            // 5. Emit SpenderAdded event
            SpenderAdded { spender }.publish(&env);
        }

        /// Removes a contract address from the approved spender whitelist.
        ///
        /// # Arguments
        /// * `admin` - The admin address (must match stored admin)
        /// * `spender` - The contract address to remove from the whitelist
        ///
        /// # Panics
        /// * If contract is not initialized
        /// * If admin does not match stored admin
        /// * If admin authentication fails
        /// * If the spender is not currently whitelisted
        ///
        /// Removes an approved spender so they can no longer call
        /// `distribute_reward`. Panics with `"Spender not found"` when
        /// attempting to remove an address that was never whitelisted,
        /// preventing silent no-ops. Decrements `SpenderCount`.
        pub fn remove_approved_spender(env: Env, admin: Address, spender: Address) {
            // 1. Fetch stored admin
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

            // 4. Assert spender exists before removing
            let spender_key = DataKey::Spender(spender.clone());
            if !env.storage().persistent().has(&spender_key) {
                panic!("Spender not found");
            }

            // 5. Remove from persistent storage
            env.storage().persistent().remove(&spender_key);

            // 6. Decrement the footprint counter
            let prev: u32 = env
                .storage()
                .instance()
                .get(&DataKey::SpenderCount)
                .unwrap_or(0);
            if prev > 0 {
                env.storage()
                    .instance()
                    .set(&DataKey::SpenderCount, &(prev - 1));
            }

            // 7. Emit SpenderRemoved event
            SpenderRemoved { spender }.publish(&env);
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
        /// breaker. Admin-only. When `IsPaused` is true,
        /// `distribute_reward` returns early with `"Contract is paused"`.
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

        /// Distributes rewards from the pool to a learner.
        ///
        /// # Arguments
        /// * `caller` - The spender contract address (must be whitelisted)
        /// * `learner` - The learner address to receive the reward
        /// * `amount` - The amount of tokens to transfer
        ///
        /// # Panics
        /// * If caller authentication fails
        /// * If amount is not positive
        /// * If caller is not an authorized spender
        /// * If contract is not initialized
        /// Performs the canonical USDC payout path used by CourseRegistry.
        /// Spender must be whitelisted via `add_approved_spender`. The
        /// amount must be strictly positive. The contract must be unpaused.
        /// Funds are transferred from this contract's balance.
        pub fn distribute_reward(env: Env, caller: Address, learner: Address, amount: i128) {
            // 0. Check if contract is paused
            let is_paused: bool = env
                .storage()
                .instance()
                .get(&DataKey::IsPaused)
                .unwrap_or(false);
            assert!(!is_paused, "Contract is paused");

            // 1. caller.require_auth()
            caller.require_auth();

            // 2. Assert amount > 0
            if amount <= 0 {
                panic!("Amount must be positive");
            }

            // 3. Check if contract is initialized first
            let token_id: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .expect("Not initialized");

            // 4. Construct DataKey::Spender(caller.clone())
            // 5. Fetch the boolean from Persistent storage. Assert it is true
            let is_authorized: bool = env
                .storage()
                .persistent()
                .get(&DataKey::Spender(caller.clone()))
                .unwrap_or(false);

            if !is_authorized {
                panic!("Caller is not an authorized spender");
            }

            // 6. Initialize token::Client::new(&env, &token_id)
            let token_client = token::Client::new(&env, &token_id);

            // 7. Call token_client.transfer(&env.current_contract_address(), &learner, &amount)
            token_client.transfer(&env.current_contract_address(), &learner, &amount);

            // 8. Emit RewardDistributed event
            RewardDistributed {
                caller,
                learner,
                amount,
            }
            .publish(&env);
        }

        /// Funds the reward pool with tokens from a donor.
        ///
        /// # Arguments
        /// * `donor` - The address donating the tokens
        /// * `amount` - The amount of tokens to donate
        ///
        /// # Panics
        /// * If contract is not initialized
        /// * If donor authentication fails
        /// * If token transfer fails
        /// Donor-funded top-up of the reward pool's token balance. The donor
        /// must authorize the token transfer; on success a `PoolFunded`
        /// event is published and the contract's balance increases.
        pub fn fund_pool(env: Env, donor: Address, amount: i128) {
            // 1. donor.require_auth()
            donor.require_auth();

            // 2. Fetch 'Token_Address' from Instance storage
            let token_id: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .expect("Not initialized");

            // 3. Initialize token::Client::new(&env, &Token_Address)
            let token_client = token::Client::new(&env, &token_id);

            // 4. Call token_client.transfer(&donor, &env.current_contract_address(), &amount)
            token_client.transfer(&donor, env.current_contract_address(), &amount);

            // 5. Emit PoolFunded event
            PoolFunded { donor, amount }.publish(&env);
        }

        /// Emergency sweep function allowing admin to transfer all tokens from the contract
        /// to a recovery wallet in case of a critical vulnerability.
        ///
        /// # Arguments
        /// * `admin` - The admin address (must match stored admin)
        /// * `recovery_wallet` - The address to receive the swept tokens
        ///
        /// # Panics
        /// * If contract is not initialized
        /// * If admin does not match stored admin
        /// * If admin authentication fails
        /// Transfers the entire token balance of the contract to a
        /// designated recovery wallet. Admin-only. Emits
        /// `EmergencySweep` with the swept amount. Intended for
        /// incidents requiring a full token rescue.
        pub fn emergency_sweep(env: Env, admin: Address, recovery_wallet: Address) {
            // 1. admin.require_auth()
            admin.require_auth();

            // 2. Fetch stored admin from Instance storage
            let stored_admin: Address = env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .expect("Not initialized");

            // 3. Assert admin == stored_admin
            if admin != stored_admin {
                panic!("Unauthorized");
            }

            // 4. Fetch token address from Instance storage
            let token_id: Address = env
                .storage()
                .instance()
                .get(&DataKey::Token)
                .expect("Not initialized");

            // 5. Initialize token client
            let token_client = token::Client::new(&env, &token_id);

            // 6. Fetch full contract token balance
            let balance = token_client.balance(&env.current_contract_address());

            // 7. Transfer full balance to recovery wallet
            token_client.transfer(&env.current_contract_address(), &recovery_wallet, &balance);

            // 8. Emit EmergencySweep event
            EmergencySweep {
                admin,
                recovery_wallet,
                amount: balance,
            }
            .publish(&env);
        }

        /// Returns the estimated number of persistent storage entries for this
        /// contract. Each whitelisted spender contributes one
        /// `DataKey::Spender(Address)` entry. The count is maintained by
        /// `add_approved_spender`.
        pub fn estimated_storage_footprint(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&DataKey::SpenderCount)
                .unwrap_or(0)
        }

        /// Upgrades the contract WASM. Only callable by the Protocol Admin.
        /// Replaces the RewardPool WASM with the supplied hash on the
        /// Soroban host. Admin-only. Emits `ContractUpgraded` on
        /// successful deployment of the new WASM.
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

            assert!(
                current_version < crate::VERSION,
                "Already at current version"
            );

            // ── v0 → v1 ──────────────────────────────────────────────────────
            // No struct changes; spender entries remain wire-compatible.
            if current_version < 1 {
                // No data transformation required.
            }

            // ── write new version ─────────────────────────────────────────────
            env.storage()
                .instance()
                .set(&DataKey::Version, &crate::VERSION);
        }

        /// Returns the schema version currently stored in instance storage.
        /// Returns 0 when the contract was deployed before versioning was introduced.
        pub fn contract_version(env: Env) -> u32 {
            env.storage().instance().get(&DataKey::Version).unwrap_or(0)
        }
    }
}

#[cfg(feature = "contract")]
pub use contract_impl::RewardPool;

mod test;
