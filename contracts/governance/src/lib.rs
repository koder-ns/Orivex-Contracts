#![no_std]

/// Current storage schema version for this contract.
/// Increment this constant when making breaking changes to stored structs or
/// DataKey variants, and add the corresponding migration step in `migrate()`.
///
/// Version history:
///   0 – pre-versioning baseline (no Version key in storage)
///   1 – initial versioned schema; Proposal struct unchanged from v0
pub const VERSION: u32 = 1;

// Operational notes — proposals progress through: created →
// voting → (executed | cancelled). Cancellation locks the
// proposal via `executed = true`. Vote weight is fetched at
// call time so badge holdings at the moment of the cast
// determine the weight.

pub const QUORUM_BASIS_POINTS: u32 = 3300;
// Crate overview — badge-weighted proposal lifecycle: create,
// vote, execute, cancel. Vote weight = number of badges owned at
// the moment of the cast.
use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, symbol_short, Address,
    BytesN, Env, Symbol, Vec,
};

pub mod types;

pub use types::{DataKey, Proposal};

const BADGE_NFT_KEY: Symbol = symbol_short!("badge");

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Badge {
    pub course_id: u32,
    pub minted_at: u64,
}

#[contractclient(name = "BadgeNFTClient")]
pub trait BadgeNFTInterface {
    fn get_badges(env: Env, learner: Address) -> Vec<Badge>;
}

#[contract]
pub struct Governance;

#[contractevent]
pub struct ProposalExecuted {
    #[topic]
    pub proposal_id: u32,
    pub proposer: Address,
}

#[contractevent]
pub struct ProposalCancelled {
    #[topic]
    pub proposal_id: u32,
    pub cancelled_by: Address,
}

#[contractevent]
pub struct ContractUpgraded {
    #[topic]
    pub admin: Address,
    pub new_wasm_hash: BytesN<32>,
}

#[contractimpl]
impl Governance {
    /// Initializes the governance contract with the admin and BadgeNFT contract address.
    /// Must be called once upon deployment.
    /// Bootstrap with admin and the BadgeNFT contract address used for
    /// vote-weight computation. The `BADGE_NFT_KEY` symbol constant
    /// names the instance slot.
    pub fn initialize(env: Env, admin: Address, badge_contract_address: Address) {
        if env.storage().instance().has(&BADGE_NFT_KEY) {
            panic!("Already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&BADGE_NFT_KEY, &badge_contract_address);
    }

    /// Returns the proposal stored for the given proposal ID.
    /// Reads a Proposal struct from persistent storage by ID. The
    /// function panics with `"Proposal not found"` when no
    /// matching `DataKey::Proposal(id)` exists.
    pub fn get_proposal(env: Env, proposal_id: u32) -> Proposal {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("Proposal not found")
    }

    /// Casts a vote on a proposal, weighted by the number of badges the voter owns.
    /// Records a voter's ballot weighted by the number of badges
    /// the voter owns at the moment of the call. Double-voting is
    /// blocked via the per-(voter, proposal_id) flag in
    /// `DataKey::UserVote`.
    pub fn cast_vote(env: Env, voter: Address, proposal_id: u32, support: bool) {
        voter.require_auth();

        let vote_key = DataKey::UserVote(voter.clone(), proposal_id);
        assert!(!env.storage().persistent().has(&vote_key), "Already voted");

        let badge_contract_address: Address = env
            .storage()
            .instance()
            .get(&BADGE_NFT_KEY)
            .expect("Contract not initialized");
        let badge_client = BadgeNFTClient::new(&env, &badge_contract_address);
        let weight = badge_client.get_badges(&voter).len();

        let mut proposal = Self::get_proposal(env.clone(), proposal_id);
        if support {
            proposal.votes_for = proposal
                .votes_for
                .checked_add(weight)
                .expect("Vote overflow");
        } else {
            proposal.votes_against = proposal
                .votes_against
                .checked_add(weight)
                .expect("Vote overflow");
        }

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        env.storage().persistent().set(&vote_key, &true);
    }

    /// Upgrades the contract WASM. Only callable by the Protocol Admin.
    /// Replaces the Governance WASM with the supplied hash on the
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
        // Proposal struct is wire-compatible between v0 and v1.
        // A future migration adding fields would iterate ProposalCount IDs
        // and rewrite each Proposal(id) here.
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

    /// Cancels an active proposal. Only callable by the proposer or the Protocol Admin.
    /// Proposer- or admin-only cancellation of an active proposal.
    /// Sets `proposal.executed = true` (the canonical "locked"
    /// state) and emits `ProposalCancelled`. Rejects cancel
    /// attempts after voting ends or after execution.
    pub fn cancel_proposal(env: Env, caller: Address, proposal_id: u32) {
        caller.require_auth();

        let mut proposal = Self::get_proposal(env.clone(), proposal_id);

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");

        assert!(
            caller == proposal.proposer || caller == stored_admin,
            "Unauthorized"
        );
        assert!(env.ledger().timestamp() < proposal.end_time, "Voting ended");
        assert!(!proposal.executed, "Already executed");

        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        ProposalCancelled {
            proposal_id,
            cancelled_by: caller,
        }
        .publish(&env);
    }

    /// Executes a proposal if it has passed and the voting period has ended.
    /// Marks the proposal as executed so the admin knows to action the approved change.
    /// Marks a passed proposal as executed if voting is closed and
    /// strictly more votes were cast in favor than against. Tied votes
    /// panic with `"Proposal rejected"`. Re-execution panics with
    /// `"Already executed"`.
    pub fn execute_proposal(env: Env, proposal_id: u32) {
        let mut proposal = Self::get_proposal(env.clone(), proposal_id);

        let total_votes = proposal.votes_for + proposal.votes_against;
        let quorum_threshold = (total_votes as u64 * QUORUM_BASIS_POINTS as u64) / 10_000;
        assert!(total_votes as u64 >= quorum_threshold, "Quorum not met");
        assert!(
            env.ledger().timestamp() > proposal.end_time,
            "Voting still active"
        );
        assert!(
            proposal.votes_for > proposal.votes_against,
            "Proposal rejected"
        );
        assert!(!proposal.executed, "Already executed");

        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        ProposalExecuted {
            proposal_id,
            proposer: proposal.proposer,
        }
        .publish(&env);
    }
}

#[cfg(test)]
mod test;
