use chrono::Utc;
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{
        AuditEvent, Authority, DependencyEdge, EdgeEndpoint, Identity, Job, MemoryObject,
        MemoryType, Review, ReviewDecision, ReviewState, content_hash, new_id,
    },
    error::AmosError,
    policy::PolicyEngine,
    store::Store,
};

const INVALIDATION_PAGE_SIZE: usize = 100;

#[derive(Clone)]
pub struct EvidenceService {
    store: Store,
    policy: PolicyEngine,
}

impl EvidenceService {
    pub fn new(store: Store, policy: PolicyEngine) -> Self {
        Self { store, policy }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn cite(
        &self,
        tenant_id: &str,
        atxn_id: &str,
        claim_id: &str,
        relation: &str,
        target_type: &str,
        target_id: &str,
        source_version: Option<String>,
    ) -> Result<DependencyEdge> {
        let mut edge = DependencyEdge {
            edge_id: new_id("edge"),
            tenant_id: tenant_id.into(),
            from: EdgeEndpoint {
                endpoint_type: "claim".into(),
                id: claim_id.into(),
            },
            relation: relation.into(),
            to: EdgeEndpoint {
                endpoint_type: target_type.into(),
                id: target_id.into(),
            },
            source_version,
            created_by_atxn: atxn_id.into(),
            content_hash: String::new(),
        };
        edge.content_hash = content_hash(&edge)?;
        Ok(edge)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn review(
        &self,
        identity: &Identity,
        artifact_id: &str,
        claim_ids: Vec<String>,
        decision: ReviewDecision,
        comment: String,
        correction: Option<Value>,
        authority: Authority,
        idempotency_key: String,
    ) -> Result<Review> {
        self.policy
            .authorize_review(identity, authority == Authority::OwnerApproved)?;
        if idempotency_key.trim().is_empty() {
            return Err(AmosError::Validation(
                "review requires an idempotency key".into(),
            ));
        }
        if claim_ids.is_empty() {
            return Err(AmosError::Validation(
                "review requires at least one claim".into(),
            ));
        }
        let mut normalized_claim_ids = claim_ids;
        normalized_claim_ids.sort();
        if normalized_claim_ids
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(AmosError::Validation(
                "review claim identifiers must be unique".into(),
            ));
        }
        let artifact = self
            .store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let transaction = self
            .store
            .get_transaction(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound(artifact.atxn_id.clone()))?;
        self.policy
            .authorize_artifact_read(identity, &artifact, &transaction)?;
        let claims = self.store.list_claims(&identity.tenant_id, artifact_id)?;
        for claim in &claims {
            self.policy
                .authorize_claim_read(identity, &transaction, claim)?;
        }
        if normalized_claim_ids
            .iter()
            .any(|id| !claims.iter().any(|claim| &claim.claim_id == id))
        {
            return Err(AmosError::NotFound("one or more claims".into()));
        }
        let request_hash = content_hash(&json!({
            "tenant_id": identity.tenant_id,
            "artifact_id": artifact_id,
            "claim_ids": normalized_claim_ids,
            "reviewer_id": identity.subject_id,
            "decision": decision,
            "comment": comment,
            "correction": correction,
            "authority": authority,
        }))?;
        let now = Utc::now();
        let review = Review {
            review_id: new_id("rev"),
            tenant_id: identity.tenant_id.clone(),
            artifact_id: artifact_id.into(),
            idempotency_key,
            request_hash,
            claim_ids: normalized_claim_ids.clone(),
            reviewer_id: identity.subject_id.clone(),
            decision,
            comment: comment.clone(),
            correction: correction.clone(),
            authority,
            effective_from: now,
            original_artifact_mutated: false,
            created_at: now,
        };
        let mut updated_claims = claims.clone();
        for claim in updated_claims
            .iter_mut()
            .filter(|claim| normalized_claim_ids.contains(&claim.claim_id))
        {
            claim.review_state = match decision {
                ReviewDecision::Approve => ReviewState::Approved,
                ReviewDecision::Reject => ReviewState::Rejected,
                ReviewDecision::Correct => ReviewState::Corrected,
            };
        }
        let mut feedback = MemoryObject::new(
            &identity.tenant_id,
            format!("feedback:{artifact_id}:{}", review.review_id),
            MemoryType::Feedback,
            format!("Payment health reviewer feedback: {comment}"),
            json!({"artifact_id":artifact_id,"claim_ids":normalized_claim_ids,"decision":decision,"correction":correction,"effective_from":review.effective_from,"role":"reviewer_feedback"}),
            "review",
            review.review_id.clone(),
            authority,
        )?;
        feedback.permissions = identity.permissions.clone();
        feedback.provenance_ref = Some(artifact_id.into());
        feedback.content_hash = content_hash(&feedback.content)?;
        self.policy.authorize_memory_write(identity, &feedback)?;
        let audit = AuditEvent {
            event_id: new_id("audit"),
            tenant_id: identity.tenant_id.clone(),
            actor_id: identity.subject_id.clone(),
            action: "review.append".into(),
            target_type: "artifact".into(),
            target_id: artifact_id.into(),
            request_id: None,
            atxn_id: Some(transaction.atxn_id),
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({"review_id":review.review_id,"decision":decision}),
            created_at: now,
        };
        let revalidation_job = Job::ready(
            &identity.tenant_id,
            "claim.revalidate",
            json!({"artifact_id":artifact_id}),
            format!("review/{}/revalidate", review.review_id),
            5,
        );
        self.store.commit_review(
            &review,
            &artifact,
            &claims,
            &updated_claims,
            &feedback,
            &audit,
            &revalidation_job,
        )
    }

    pub fn invalidate_memory(
        &self,
        tenant_id: &str,
        object_id: &str,
        reason: &str,
    ) -> Result<Vec<String>> {
        let deduplication_key = format!(
            "manual/{object_id}/{}",
            content_hash(&json!({"reason":reason}))?
        );
        self.invalidate_memory_with_key(tenant_id, object_id, reason, &deduplication_key)
    }

    pub fn invalidate_memory_with_key(
        &self,
        tenant_id: &str,
        object_id: &str,
        reason: &str,
        deduplication_key: &str,
    ) -> Result<Vec<String>> {
        self.store.invalidate_claims_page(
            tenant_id,
            "memory",
            object_id,
            reason,
            deduplication_key,
            INVALIDATION_PAGE_SIZE,
        )
    }
}
