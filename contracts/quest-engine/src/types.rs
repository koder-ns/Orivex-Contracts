use soroban_sdk::{contracttype, Address, BytesN};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuestType {
    Build,
    Explore,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Quest {
    pub employer: Address,
    pub reward_amount: i128,
    pub quest_type: QuestType,
    pub metadata_hash: BytesN<32>,
    pub active: bool,
}

/// Schema used before VERSION 1 was introduced (v0 layout).
/// Wire-compatible with `Quest`; kept for documentation clarity.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestV0 {
    pub employer: Address,
    pub reward_amount: i128,
    pub quest_type: QuestType,
    pub metadata_hash: BytesN<32>,
    pub active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmissionStatus {
    Pending,
    Approved,
    Rejected,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submission {
    pub proof_hash: BytesN<32>,
    pub status: SubmissionStatus,
}

/// Status of a learner's explore-quest submission.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExploreSubmissionStatus {
    /// Submitted by the learner, awaiting admin decision.
    Pending,
    /// Admin called `verify_explore_quest` — reward has been distributed.
    Verified,
    /// Admin called `reject_explore_quest` — no reward issued.
    Rejected,
}

/// On-chain record of a learner's explore-quest proof submission.
///
/// Stored under `DataKey::ExploreSubmission(learner, quest_id)`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExploreSubmission {
    /// Hash of the off-chain proof artifact supplied by the learner.
    pub proof_hash: BytesN<32>,
    pub status: ExploreSubmissionStatus,
}

/// Maximum byte-length of the rejection reason string.
/// Keeps Soroban string-cost predictable and prevents state bloat.
pub const MAX_REASON_LEN: u32 = 256;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Quest(u32),
    Submission(Address, u32),        // Build-quest (submitter, quest_id)
    ExploreSubmission(Address, u32), // Explore-quest (learner, quest_id)
    Token,
    QuestCounter,
    RewardPool,
    IsPaused,
    StakeVault,
    /// Monotonically increasing schema version stored in instance storage.
    /// 0  = pre-versioning (no Version key present).
    /// 1  = current schema (this build).
    Version,
}
