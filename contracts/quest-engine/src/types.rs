use soroban_sdk::{contracttype, Address, BytesN};

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
    pub reviewed_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dispute {
    pub quest_id: u32,
    pub learner: Address,
    pub reason: BytesN<32>,
    pub opened_at: u64,
    pub resolved: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Quest(u32),
    Submission(Address, u32), // (Submitter Address, Quest ID)
    Token,
    QuestCounter,
    RewardPool,
    IsPaused,
    StakeVault,
    /// Monotonically increasing schema version stored in instance storage.
    /// 0  = pre-versioning (no Version key present).
    /// 1  = current schema (this build).
    Version,
    Dispute(u32), // Dispute ID
    DisputeCounter,
    Governance,
}
