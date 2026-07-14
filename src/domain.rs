use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

pub fn content_hash<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    ActiveContext,
    DataState,
    StreamState,
    Schema,
    SemanticDefinition,
    Document,
    PriorAnalysis,
    Feedback,
    PermissionPolicy,
    ReviewPolicy,
    Execution,
    Artifact,
    Claim,
    Provenance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    UntrustedExternal,
    ModelHypothesis,
    UserNote,
    SystemObserved,
    ReviewerApproved,
    OwnerApproved,
}

impl Authority {
    pub const fn rank(self) -> u8 {
        match self {
            Self::UntrustedExternal => 0,
            Self::ModelHypothesis => 1,
            Self::UserNote => 2,
            Self::SystemObserved => 3,
            Self::ReviewerApproved => 4,
            Self::OwnerApproved => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Staging,
    Active,
    Superseded,
    Revoked,
    Rejected,
    PendingReview,
    Tombstoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryObject {
    pub tenant_id: String,
    pub object_id: String,
    pub logical_key: String,
    pub memory_type: MemoryType,
    pub summary: String,
    pub content: Value,
    pub external_ref: Option<String>,
    pub source_id: String,
    pub source_version: String,
    pub authority: Authority,
    pub effective_start: Option<DateTime<Utc>>,
    pub effective_end: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
    pub permissions: BTreeSet<String>,
    pub sensitivity: String,
    pub version: String,
    pub status: MemoryStatus,
    pub supersedes: Vec<String>,
    pub superseded_by: Option<String>,
    pub provenance_ref: Option<String>,
    pub content_hash: String,
    pub governing: bool,
}

impl MemoryObject {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: impl Into<String>,
        logical_key: impl Into<String>,
        memory_type: MemoryType,
        summary: impl Into<String>,
        content: Value,
        source_id: impl Into<String>,
        source_version: impl Into<String>,
        authority: Authority,
    ) -> Self {
        let content_hash = content_hash(&content);
        Self {
            tenant_id: tenant_id.into(),
            object_id: new_id("mem"),
            logical_key: logical_key.into(),
            memory_type,
            summary: summary.into(),
            content,
            external_ref: None,
            source_id: source_id.into(),
            source_version: source_version.into(),
            authority,
            effective_start: None,
            effective_end: None,
            recorded_at: Utc::now(),
            permissions: BTreeSet::new(),
            sensitivity: "internal".into(),
            version: "1".into(),
            status: MemoryStatus::Active,
            supersedes: vec![],
            superseded_by: None,
            provenance_ref: None,
            content_hash,
            governing: true,
        }
    }

    pub fn effective_at(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.effective_start.is_none_or(|value| value <= end)
            && self.effective_end.is_none_or(|value| value >= start)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub tenant_id: String,
    pub subject_id: String,
    pub roles: BTreeSet<String>,
    pub groups: BTreeSet<String>,
    pub permissions: BTreeSet<String>,
    pub policy_attributes: BTreeMap<String, String>,
    pub policy_epoch: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConsistencyClass {
    C0,
    C1,
    C2,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Exploratory,
    Internal,
    MaterialInternal,
    External,
    Regulated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Budgets {
    pub max_context_items: usize,
    pub max_context_tokens: usize,
    pub max_steps: u32,
    pub max_repairs: u32,
    pub max_rows: u64,
    pub max_bytes: u64,
    pub max_seconds: u64,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_context_items: 16,
            max_context_tokens: 8_000,
            max_steps: 8,
            max_repairs: 2,
            max_rows: 50_000,
            max_bytes: 5_000_000,
            max_seconds: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefinition {
    pub tenant_id: String,
    pub task_type: String,
    pub version: u32,
    pub status: String,
    pub risk_class: RiskClass,
    pub required_roles: BTreeMap<String, MemoryType>,
    pub optional_roles: BTreeMap<String, MemoryType>,
    pub minimum_consistency: BTreeMap<String, ConsistencyClass>,
    pub allowed_tools: BTreeSet<String>,
    pub claim_types: BTreeSet<String>,
    pub verifier_profile: String,
    pub publication_policy: String,
    pub budgets: Budgets,
    pub artifact_schema: String,
    pub effective_start: Option<DateTime<Utc>>,
    pub effective_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AtxnState {
    Admitted,
    Observing,
    Selecting,
    Planning,
    Executing,
    Repairing,
    Composing,
    Verifying,
    Revalidating,
    EvidenceCommitted,
    ObjectFinalizing,
    ObjectFailed,
    PublicationPending,
    Published,
    PublicationFailed,
    RevocationPending,
    Revoked,
    NeedsReview,
    Rejected,
    Aborted,
}

impl AtxnState {
    pub fn can_transition(self, next: Self) -> bool {
        use AtxnState::*;
        matches!(
            (self, next),
            (Admitted, Observing | Rejected)
                | (Observing, Selecting | NeedsReview | Aborted)
                | (Selecting, Planning | NeedsReview | Rejected)
                | (Planning, Executing | NeedsReview | Rejected)
                | (Executing, Composing | Repairing | Aborted)
                | (Repairing, Executing | NeedsReview | Rejected)
                | (Composing, Verifying | Aborted)
                | (Verifying, Revalidating | Repairing | NeedsReview | Rejected)
                | (
                    Revalidating,
                    EvidenceCommitted | Repairing | NeedsReview | Rejected | Aborted
                )
                | (
                    EvidenceCommitted,
                    ObjectFinalizing | PublicationPending | NeedsReview
                )
                | (ObjectFinalizing, PublicationPending | ObjectFailed)
                | (PublicationPending, Published | PublicationFailed)
                | (Published, RevocationPending)
                | (RevocationPending, Revoked | PublicationFailed)
                | (NeedsReview, Verifying | Revalidating | Rejected)
        )
    }

    pub fn terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Aborted | Self::Revoked)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Warning,
    Repair,
    NeedsReview,
    Reject,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalyticalTransaction {
    pub tenant_id: String,
    pub atxn_id: String,
    pub request_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub subject_id: String,
    pub request: String,
    pub task_type: String,
    pub task_version: u32,
    pub risk_class: RiskClass,
    pub budgets: Budgets,
    pub policy_epoch: u64,
    pub source_versions: BTreeMap<String, String>,
    pub state: AtxnState,
    pub state_seq: u64,
    pub terminal: bool,
    pub outcome: Option<Outcome>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextOmission {
    pub role: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextConflict {
    pub logical_key: String,
    pub object_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextManifest {
    pub manifest_id: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub task_definition: String,
    pub policy_epoch: u64,
    pub required_role_coverage: BTreeMap<String, Vec<String>>,
    pub optional_selected: Vec<String>,
    pub omissions: Vec<ContextOmission>,
    pub conflicts: Vec<ContextConflict>,
    pub token_count: usize,
    pub source_versions: BTreeMap<String, String>,
    pub selected_objects: Vec<MemoryObject>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationLimits {
    pub seconds: u64,
    pub rows: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub step_id: String,
    pub purpose: String,
    pub tool: String,
    pub source_id: String,
    pub input_object_ids: Vec<String>,
    pub parameter_schema: String,
    pub parameters: Value,
    pub expected_output_schema: String,
    pub limits: OperationLimits,
    pub max_attempts: u32,
    pub repair_classes: BTreeSet<String>,
    pub verifier_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TypedPlan {
    pub plan_id: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub task_definition: String,
    pub manifest_id: String,
    pub model_identity: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityLimits {
    pub seconds: u64,
    pub rows: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityClaims {
    pub issuer: String,
    pub audience: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub plan_id: String,
    pub step_id: String,
    pub subject_id: String,
    pub tool: String,
    pub source_id: String,
    pub operations: BTreeSet<String>,
    pub relations: BTreeSet<String>,
    pub limits: CapabilityLimits,
    pub policy_epoch: u64,
    pub fencing_token: u64,
    pub token_id: String,
    pub not_before: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityEnvelope {
    pub claims: CapabilityClaims,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionRecord {
    pub execution_id: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub plan_id: String,
    pub step_id: String,
    pub tool: String,
    pub tool_version: String,
    pub capability_id: String,
    pub parameters: Value,
    pub parameters_hash: String,
    pub input_versions: BTreeMap<String, String>,
    pub output: Value,
    pub output_hash: String,
    pub row_count: u64,
    pub byte_count: u64,
    pub latency_ms: u64,
    pub fencing_token: u64,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationCheck {
    pub rule_id: String,
    pub outcome: Outcome,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationRecord {
    pub verification_id: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub execution_id: Option<String>,
    pub verifier_profile: String,
    pub profile_version: u32,
    pub outcome: Outcome,
    pub checks: Vec<VerificationCheck>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub permitted_repair: Option<String>,
    pub input_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SqlPreflight {
    pub manifest_id: String,
    pub referenced_versions: BTreeMap<String, String>,
    pub verification: VerificationRecord,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicationValidity {
    Draft,
    ValidAtPublication,
    PublicationFailed,
    Revoked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticValidity {
    Current,
    PendingRevalidation,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVisibility {
    Allowed,
    Redacted,
    Denied,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAvailability {
    Level0,
    Level1,
    Level2,
    Level3,
    Level4,
    Degraded,
    Expired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Unreviewed,
    NeedsReview,
    Verified,
    Approved,
    Corrected,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SupersessionState {
    Active,
    Superseded,
    Tombstoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Claim {
    pub tenant_id: String,
    pub claim_id: String,
    pub artifact_id: String,
    pub claim_type: String,
    pub text: String,
    pub payload: Value,
    pub risk_class: RiskClass,
    pub support_execution_ids: Vec<String>,
    pub verification_ids: Vec<String>,
    pub publication_validity: PublicationValidity,
    pub semantic_validity: SemanticValidity,
    pub policy_visibility: PolicyVisibility,
    pub replay_availability: ReplayAvailability,
    pub review_state: ReviewState,
    pub supersession_state: SupersessionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeEndpoint {
    pub endpoint_type: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyEdge {
    pub edge_id: String,
    pub tenant_id: String,
    pub from: EdgeEndpoint,
    pub relation: String,
    pub to: EdgeEndpoint,
    pub source_version: Option<String>,
    pub created_by_atxn: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub tenant_id: String,
    pub artifact_id: String,
    pub atxn_id: String,
    pub artifact_type: String,
    pub title: String,
    pub content: String,
    pub content_hash: String,
    pub audience: String,
    pub risk_class: RiskClass,
    pub object_state: String,
    pub publication_validity: PublicationValidity,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplayPackage {
    pub package_id: String,
    pub tenant_id: String,
    pub artifact_id: String,
    pub replay_level: u8,
    pub manifest_id: String,
    pub plan_id: String,
    pub execution_ids: Vec<String>,
    pub template: String,
    pub render_config_hash: String,
    pub retained_until: DateTime<Utc>,
    pub expected_artifact_hash: String,
    pub expected_execution_hashes: BTreeMap<String, String>,
    pub source_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
    Correct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Review {
    pub review_id: String,
    pub tenant_id: String,
    pub artifact_id: String,
    pub claim_ids: Vec<String>,
    pub reviewer_id: String,
    pub decision: ReviewDecision,
    pub comment: String,
    pub correction: Option<Value>,
    pub authority: Authority,
    pub effective_from: DateTime<Utc>,
    pub original_artifact_mutated: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewResult {
    pub review: Review,
    pub transaction: AnalyticalTransaction,
    pub artifact: Artifact,
    pub claims: Vec<Claim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    pub event_id: String,
    pub tenant_id: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub request_id: Option<String>,
    pub atxn_id: Option<String>,
    pub outcome: String,
    pub policy_epoch: u64,
    pub details: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Ready,
    Running,
    RetryWait,
    Complete,
    DeadLetter,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub job_id: String,
    pub tenant_id: String,
    pub job_type: String,
    pub payload: Value,
    pub idempotency_key: String,
    pub state: JobState,
    pub attempt: u32,
    pub max_attempts: u32,
    pub fencing_token: u64,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub next_run_at: DateTime<Utc>,
    pub dead_letter_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceObservation {
    pub tenant_id: String,
    pub source_id: String,
    pub reference: String,
    pub source_version: String,
    pub observed_at: DateTime<Utc>,
    pub effective_start: Option<DateTime<Utc>>,
    pub effective_end: Option<DateTime<Utc>>,
    pub freshness_seconds: u64,
    pub sensitivity: String,
    pub permissions: BTreeSet<String>,
    pub consistency_class: ConsistencyClass,
    pub retention_deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceEvent {
    pub event_id: String,
    pub tenant_id: String,
    pub source_id: String,
    pub subject: String,
    pub previous_version: Option<String>,
    pub current_version: Option<String>,
    pub change_kind: String,
    pub occurred_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub cursor: String,
    pub deduplication_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorHealth {
    pub source_id: String,
    pub status: String,
    pub lag_seconds: u64,
    pub rate_limit_remaining: Option<u64>,
    pub degraded_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunResult {
    pub transaction: AnalyticalTransaction,
    pub manifest: ContextManifest,
    pub plan: TypedPlan,
    pub executions: Vec<ExecutionRecord>,
    pub verifications: Vec<VerificationRecord>,
    pub artifact: Artifact,
    pub claims: Vec<Claim>,
    pub dependencies: Vec<DependencyEdge>,
    pub replay_package: ReplayPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayResult {
    pub artifact_id: String,
    pub status: Outcome,
    pub matching_execution_ids: Vec<String>,
    pub changed_execution_ids: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::AtxnState::*;

    #[test]
    fn state_machine_matches_normative_contract() {
        assert!(Admitted.can_transition(Observing));
        assert!(Executing.can_transition(Repairing));
        assert!(NeedsReview.can_transition(Revalidating));
        assert!(Published.can_transition(RevocationPending));
        assert!(!Published.can_transition(Executing));
        assert!(!Rejected.can_transition(Admitted));
    }
}
