use chrono::Utc;
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{
        AuditEvent, Authority, EdgeEndpoint, Identity, MemoryObject, MemoryType, Review,
        ReviewDecision, ReviewState, SemanticValidity, content_hash, new_id,
    },
    error::AmosError,
    memory::MemoryService,
    policy::PolicyEngine,
    scheduler::Scheduler,
    store::Store,
};

#[derive(Clone)]
pub struct EvidenceService {
    store: Store,
    memory: MemoryService,
    policy: PolicyEngine,
    scheduler: Scheduler,
}

impl EvidenceService {
    pub fn new(
        store: Store,
        memory: MemoryService,
        policy: PolicyEngine,
        scheduler: Scheduler,
    ) -> Self {
        Self {
            store,
            memory,
            policy,
            scheduler,
        }
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
    ) -> crate::domain::DependencyEdge {
        let mut edge = crate::domain::DependencyEdge {
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
        edge.content_hash = content_hash(&edge);
        edge
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
    ) -> Result<Review> {
        self.policy
            .authorize_review(identity, authority == Authority::OwnerApproved)?;
        self.store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let mut claims = self.store.list_claims(&identity.tenant_id, artifact_id)?;
        if claim_ids
            .iter()
            .any(|id| !claims.iter().any(|claim| &claim.claim_id == id))
        {
            return Err(AmosError::NotFound("one or more claims".into()));
        }
        let review = Review {
            review_id: new_id("rev"),
            tenant_id: identity.tenant_id.clone(),
            artifact_id: artifact_id.into(),
            claim_ids: claim_ids.clone(),
            reviewer_id: identity.subject_id.clone(),
            decision,
            comment: comment.clone(),
            correction: correction.clone(),
            authority,
            effective_from: Utc::now(),
            original_artifact_mutated: false,
            created_at: Utc::now(),
        };
        self.store.save_review(&review)?;
        for claim in claims
            .iter_mut()
            .filter(|claim| claim_ids.contains(&claim.claim_id))
        {
            claim.review_state = match decision {
                ReviewDecision::Approve => ReviewState::Approved,
                ReviewDecision::Reject => ReviewState::Rejected,
                ReviewDecision::Correct => ReviewState::Corrected,
            };
            self.store.update_claim(claim)?;
        }
        let mut feedback = MemoryObject::new(
            &identity.tenant_id,
            format!("feedback:{artifact_id}:{}", review.review_id),
            MemoryType::Feedback,
            format!("Payment health reviewer feedback: {comment}"),
            json!({"artifact_id":artifact_id,"claim_ids":claim_ids,"decision":decision,"correction":correction,"role":"reviewer_feedback"}),
            "review",
            review.review_id.clone(),
            authority,
        );
        feedback.permissions = identity.permissions.clone();
        feedback.provenance_ref = Some(artifact_id.into());
        feedback.content_hash = content_hash(&feedback.content);
        self.memory.write(identity, &feedback)?;
        self.store.append_audit(&AuditEvent {
            event_id: new_id("audit"),
            tenant_id: identity.tenant_id.clone(),
            actor_id: identity.subject_id.clone(),
            action: "review.append".into(),
            target_type: "artifact".into(),
            target_id: artifact_id.into(),
            request_id: None,
            atxn_id: None,
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({"review_id":review.review_id,"decision":decision}),
            created_at: Utc::now(),
        })?;
        self.scheduler.enqueue(
            &identity.tenant_id,
            "claim.revalidate",
            json!({"artifact_id":artifact_id}),
            format!("review/{}/revalidate", review.review_id),
        )?;
        Ok(review)
    }

    pub fn invalidate_memory(
        &self,
        tenant_id: &str,
        object_id: &str,
        reason: &str,
    ) -> Result<Vec<String>> {
        let edges = self.store.list_edges_to(tenant_id, "memory", object_id)?;
        let mut affected = vec![];
        for edge in edges {
            let Some(mut claim) = self.store.get_claim(tenant_id, &edge.from.id)? else {
                continue;
            };
            claim.semantic_validity = SemanticValidity::PendingRevalidation;
            self.store.update_claim(&claim)?;
            affected.push(claim.claim_id.clone());
            self.scheduler.enqueue(
                tenant_id,
                "claim.revalidate",
                json!({"claim_id":claim.claim_id,"artifact_id":claim.artifact_id,"reason":reason}),
                format!("invalidate/{}/{}/{}", object_id, claim.claim_id, reason),
            )?;
        }
        Ok(affected)
    }
}
