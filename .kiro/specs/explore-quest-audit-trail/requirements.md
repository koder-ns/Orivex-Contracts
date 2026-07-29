# Requirements Document

## Introduction

The `quest-engine` contract currently supports Explore quests as a light-weight, off-chain-verified variant of the Build quest flow. An admin calls `verify_explore_quest`, which immediately triggers a reward payout from the RewardPool — there is no submission step, no rejection path, and no on-chain record of a refusal. This means:

- Learners leave no trace before an admin decision, making audits and dispute resolution impossible.
- Admins have no canonical way to record a rejection; any refusal is invisible on-chain.
- There is no event analogue to the Build quest's `ProofSubmitted` / `SubmissionReviewed` pair.

This feature adds three building blocks to close those gaps:

1. An optional **learner submission step** (`submit_explore_proof`) that mirrors the Build quest flow and anchors intent on-chain.
2. An explicit **admin rejection path** (`reject_explore_quest`) that records refusal with a bounded reason string and emits a rejection event.
3. Full **event observability** for both new operations, consistent with the existing `ExploreQuestVerified` event.

The result is a complete off-chain → on-chain lifecycle for Explore quests, enabling dispute handling per Issue #34.

---

## Glossary

- **QuestEngine**: The Soroban smart contract (`quest-engine`) that manages Build and Explore quest lifecycles, reward payouts, and audit events.
- **Admin**: The privileged address stored in `DataKey::Admin` at initialization time; only this address may create Explore quests, verify them, or reject them.
- **Learner**: Any address that participates in a quest as the reward recipient.
- **Explore_Quest**: A `Quest` with `quest_type == QuestType::Explore`, funded by the RewardPool rather than an employer escrow.
- **Build_Quest**: A `Quest` with `quest_type == QuestType::Build`, funded directly by an employer.
- **Proof_Hash**: A `BytesN<32>` that is a content-addressing hash of off-chain evidence (e.g., SHA-256 of a GitHub commit URL or a screenshot bundle).
- **ExploreSubmission**: The on-chain record persisted for a learner's intent to complete a specific Explore quest, stored under `DataKey::ExploreSubmission(learner, quest_id)`.
- **Rejection_Reason**: A `soroban_sdk::String` of at most `MAX_EXPLORE_REJECTION_REASON_LEN` bytes, supplied by the admin when rejecting an Explore quest attempt. Bounded to limit ledger entry growth.
- **MAX_EXPLORE_REJECTION_REASON_LEN**: A constant (`u32`) set to `256`, representing the maximum UTF-8 byte length permitted for a `Rejection_Reason` string.
- **ExploreProofSubmitted**: A `#[contractevent]` emitted by `submit_explore_proof`, analogous to the Build quest's `ProofSubmitted` event.
- **ExploreQuestRejected**: A `#[contractevent]` emitted by `reject_explore_quest`, recording the admin decision to refuse a learner's Explore quest attempt.
- **ExploreQuestVerified**: The existing `#[contractevent]` emitted by `verify_explore_quest`; unchanged by this feature.
- **DataKey**: The storage key enum in `types.rs`; extended with `ExploreSubmission(Address, u32)` by this feature.
- **SubmissionStatus**: The existing enum (`Pending`, `Approved`, `Rejected`) reused for `ExploreSubmission.status`.
- **RewardPool**: The cross-contract interface whose `distribute_reward` is called by `verify_explore_quest`; unchanged by this feature.
- **Issue #34**: The Orivex project's tracked dispute-handling issue, which requires on-chain evidence of both approval and rejection decisions.

---

## Requirements

### Requirement 1: Learner Explore Proof Submission

**User Story:** As a learner, I want to submit an on-chain proof record for an Explore quest before the admin reviews my work, so that my intent is permanently anchored to the blockchain and available for dispute resolution.

#### Acceptance Criteria

1. WHEN a learner calls `submit_explore_proof(env, learner, quest_id, proof_hash)`, THE QuestEngine SHALL authenticate the learner via `learner.require_auth()`.
2. WHEN a learner calls `submit_explore_proof`, THE QuestEngine SHALL verify that the quest identified by `quest_id` exists in persistent storage; IF the quest does not exist, THEN THE QuestEngine SHALL panic with the message `"Quest not found"`.
3. WHEN a learner calls `submit_explore_proof`, THE QuestEngine SHALL verify that the identified quest has `quest_type == QuestType::Explore`; IF the quest type is not `Explore`, THEN THE QuestEngine SHALL panic with the message `"Only Explore quests accept explore proofs"`.
4. WHEN a learner calls `submit_explore_proof`, THE QuestEngine SHALL verify that the identified quest is active; IF `quest.active == false`, THEN THE QuestEngine SHALL panic with the message `"Quest is not active"`.
5. WHEN a learner calls `submit_explore_proof` and an `ExploreSubmission` already exists under `DataKey::ExploreSubmission(learner, quest_id)`, THEN THE QuestEngine SHALL panic with the message `"Explore submission already exists"`.
6. WHEN a learner calls `submit_explore_proof` and all validations pass, THE QuestEngine SHALL persist an `ExploreSubmission { proof_hash, status: SubmissionStatus::Pending }` to persistent storage under `DataKey::ExploreSubmission(learner, quest_id)`.
7. WHEN a learner calls `submit_explore_proof` and the submission is saved, THE QuestEngine SHALL emit an `ExploreProofSubmitted { learner, quest_id, proof_hash }` event.

### Requirement 2: Admin Explore Quest Rejection

**User Story:** As an admin, I want to explicitly reject a learner's Explore quest attempt with a bounded reason string, so that the refusal decision is permanently recorded on-chain and visible to all parties.

#### Acceptance Criteria

1. WHEN the admin calls `reject_explore_quest(env, admin, learner, quest_id, reason)`, THE QuestEngine SHALL authenticate the caller via `admin.require_auth()`.
2. WHEN the admin calls `reject_explore_quest`, THE QuestEngine SHALL verify that `admin` matches the address stored under `DataKey::Admin`; IF they do not match, THEN THE QuestEngine SHALL panic with the message `"Unauthorized"`.
3. WHEN the admin calls `reject_explore_quest`, THE QuestEngine SHALL verify that the `reason` string's byte length is at most `MAX_EXPLORE_REJECTION_REASON_LEN` (256); IF the length exceeds this limit, THEN THE QuestEngine SHALL panic with the message `"Reason too long"`.
4. WHEN the admin calls `reject_explore_quest`, THE QuestEngine SHALL verify that the quest identified by `quest_id` exists; IF it does not, THEN THE QuestEngine SHALL panic with the message `"Quest not found"`.
5. WHEN the admin calls `reject_explore_quest`, THE QuestEngine SHALL verify that the identified quest has `quest_type == QuestType::Explore`; IF it does not, THEN THE QuestEngine SHALL panic with the message `"Not an Explore quest"`.
6. WHEN the admin calls `reject_explore_quest` and a prior `ExploreSubmission` exists under `DataKey::ExploreSubmission(learner, quest_id)` with `status == SubmissionStatus::Pending`, THE QuestEngine SHALL update the stored `ExploreSubmission.status` to `SubmissionStatus::Rejected`.
7. WHEN the admin calls `reject_explore_quest` and all validations pass, THE QuestEngine SHALL emit an `ExploreQuestRejected { admin, learner, quest_id, reason }` event regardless of whether a prior on-chain `ExploreSubmission` exists (to support rejecting learners who submitted off-chain only).
8. WHEN the admin calls `reject_explore_quest`, THE QuestEngine SHALL NOT transfer any tokens or interact with the RewardPool.

### Requirement 3: Explore Submission Query

**User Story:** As an integrator or front-end developer, I want to query the on-chain status of a learner's Explore quest proof submission, so that I can display the current state without reading raw storage.

#### Acceptance Criteria

1. THE QuestEngine SHALL expose a `get_explore_submission(env, learner, quest_id)` function that returns `Option<ExploreSubmission>`.
2. WHEN `get_explore_submission` is called for a `(learner, quest_id)` pair that has no record in persistent storage, THE QuestEngine SHALL return `None`.
3. WHEN `get_explore_submission` is called for a `(learner, quest_id)` pair that has a persisted `ExploreSubmission`, THE QuestEngine SHALL return `Some(ExploreSubmission)` containing the current `proof_hash` and `status`.

### Requirement 4: Verify Explore Quest Requires a Prior Pending Submission (Optional Gate)

**User Story:** As a protocol designer, I want `verify_explore_quest` to optionally enforce that the learner has an on-chain `ExploreSubmission` in `Pending` status before paying out, so that the full audit trail is always complete when the submission step is used.

#### Acceptance Criteria

1. WHEN `verify_explore_quest(env, admin, learner, quest_id)` is called and a `DataKey::ExploreSubmission(learner, quest_id)` entry exists with `status == SubmissionStatus::Pending`, THE QuestEngine SHALL update the stored `ExploreSubmission.status` to `SubmissionStatus::Approved` before triggering the RewardPool payout.
2. WHEN `verify_explore_quest` is called and no `DataKey::ExploreSubmission(learner, quest_id)` exists, THE QuestEngine SHALL proceed with the RewardPool payout without requiring a prior submission (backward-compatible behaviour).
3. WHEN `verify_explore_quest` is called and a `DataKey::ExploreSubmission(learner, quest_id)` entry exists with `status != SubmissionStatus::Pending`, THEN THE QuestEngine SHALL panic with the message `"Explore submission is not pending"`.

### Requirement 5: Bounded String Cost Awareness

**User Story:** As a smart-contract developer, I want the `reason` parameter in `reject_explore_quest` to be bounded to 256 UTF-8 bytes, so that ledger entry growth and Soroban resource fees remain predictable and bounded.

#### Acceptance Criteria

1. THE QuestEngine SHALL define a constant `MAX_EXPLORE_REJECTION_REASON_LEN` with the value `256u32` in `lib.rs`.
2. WHEN `reject_explore_quest` receives a `reason` whose byte length is exactly `MAX_EXPLORE_REJECTION_REASON_LEN`, THE QuestEngine SHALL accept it without error.
3. WHEN `reject_explore_quest` receives a `reason` whose byte length is one byte greater than `MAX_EXPLORE_REJECTION_REASON_LEN` (i.e., 257 bytes), THEN THE QuestEngine SHALL panic with the message `"Reason too long"`.
4. WHEN `reject_explore_quest` receives an empty `reason` string (0 bytes), THE QuestEngine SHALL accept it without error.

### Requirement 6: New Storage Keys and Types

**User Story:** As a contract developer, I want clearly typed storage keys and event structs for the new Explore audit-trail operations, so that storage layout is explicit and events are well-structured.

#### Acceptance Criteria

1. THE QuestEngine's `DataKey` enum SHALL include a new variant `ExploreSubmission(Address, u32)` to store per-learner Explore quest submission records in persistent storage.
2. THE QuestEngine's `lib.rs` SHALL define a `#[contractevent]` struct `ExploreProofSubmitted` with fields: `#[topic] learner: Address`, `#[topic] quest_id: u32`, and `proof_hash: BytesN<32>`.
3. THE QuestEngine's `lib.rs` SHALL define a `#[contractevent]` struct `ExploreQuestRejected` with fields: `#[topic] admin: Address`, `#[topic] learner: Address`, `#[topic] quest_id: u32`, and `reason: soroban_sdk::String`.
4. THE QuestEngine's reused `ExploreSubmission` storage type SHALL use the existing `Submission` struct (`proof_hash: BytesN<32>`, `status: SubmissionStatus`), avoiding duplication of storage types.

### Requirement 7: Test Coverage

**User Story:** As a contributor, I want test coverage for the full submission and rejection lifecycle, so that regressions are caught and the feature behaves correctly under all expected conditions.

#### Acceptance Criteria

1. THE test suite SHALL include a test verifying that `submit_explore_proof` succeeds for a valid Explore quest and that the stored `ExploreSubmission` has `status == Pending`.
2. THE test suite SHALL include a test verifying that `submit_explore_proof` emits an `ExploreProofSubmitted` event.
3. THE test suite SHALL include a test verifying that `submit_explore_proof` panics with `"Explore submission already exists"` when called twice for the same `(learner, quest_id)` pair.
4. THE test suite SHALL include a test verifying that `submit_explore_proof` panics with `"Only Explore quests accept explore proofs"` when the target quest is a Build quest.
5. THE test suite SHALL include a test verifying that `reject_explore_quest` succeeds when called by the admin, emits `ExploreQuestRejected`, and updates the stored submission status to `Rejected` when a prior `ExploreSubmission` exists.
6. THE test suite SHALL include a test verifying that `reject_explore_quest` panics with `"Unauthorized"` when called by a non-admin address.
7. THE test suite SHALL include a test verifying that `reject_explore_quest` panics with `"Reason too long"` when the `reason` string exceeds `MAX_EXPLORE_REJECTION_REASON_LEN` bytes.
8. THE test suite SHALL include a test verifying that `reject_explore_quest` emits `ExploreQuestRejected` even when no prior `ExploreSubmission` exists (off-chain-only submission path).
9. THE test suite SHALL include a test verifying that `verify_explore_quest` updates the stored `ExploreSubmission.status` to `Approved` when a prior `Pending` submission exists.
10. THE test suite SHALL include a test verifying that `verify_explore_quest` panics with `"Explore submission is not pending"` when the stored submission status is already `Rejected`.
11. THE test suite SHALL include a test verifying that `get_explore_submission` returns `None` for an unknown `(learner, quest_id)` pair.

### Requirement 8: Documentation — Off-Chain to On-Chain Lifecycle

**User Story:** As a developer integrating against the QuestEngine, I want the README to explain the full Explore quest lifecycle from off-chain completion to on-chain verification or rejection, so that I can correctly sequence contract calls.

#### Acceptance Criteria

1. THE QuestEngine's `README.md` SHALL include a prose section titled `"Explore Quest Lifecycle"` that describes the sequence: (a) admin creates Explore quest via `create_explore_quest`, (b) learner optionally calls `submit_explore_proof` to anchor intent on-chain, (c) admin either calls `verify_explore_quest` to pay out or `reject_explore_quest` to refuse with a reason, (d) all state transitions emit observable events.
2. THE QuestEngine's `README.md` SHALL list `submit_explore_proof`, `reject_explore_quest`, and `get_explore_submission` in the Functions section with brief descriptions.
3. THE QuestEngine's `README.md` SHALL reference Issue #34 as the upstream dispute-handling context for the rejection flow.
