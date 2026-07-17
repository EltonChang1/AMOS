use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{Duration, Utc};
use serde_json::{Value, json};

use crate::{
    Result,
    connectors::{Connector, SqliteWarehouseConnector},
    context::ContextCompiler,
    domain::{
        AnalyticalTransaction, Artifact, AtxnState, AuditEvent, Authority, Claim, ContextManifest,
        DependencyEdge, EdgeEndpoint, ExecutionRecord, Identity, Job, MemoryStatus, OutboxEvent,
        Outcome, PlanStep, PolicyVisibility, PublicationValidity, ReplayAvailability,
        ReplayPackage, ReplayResult, Review, ReviewDecision, ReviewResult, ReviewState, RiskClass,
        RunResult, SemanticValidity, SqlPreflight, SupersessionState, TypedPlan, content_hash,
        new_id,
    },
    error::AmosError,
    evidence::EvidenceService,
    memory::MemoryService,
    policy::PolicyEngine,
    scheduler::Scheduler,
    seed::{SOURCE, SPIKE_START, TENANT, WINDOW_END, WINDOW_START},
    store::Store,
    verification::Verifier,
    workers::{CapabilityIssuer, ChartWorker, SqlWorker, StatisticsWorker},
};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub control_db: PathBuf,
    pub warehouse_db: PathBuf,
    pub capability_key: Vec<u8>,
}
impl RuntimeConfig {
    pub fn local(root: impl AsRef<Path>) -> Self {
        Self {
            control_db: root.as_ref().join("data/amos.sqlite"),
            warehouse_db: root.as_ref().join("data/payments.sqlite"),
            capability_key: b"development-only-capability-key-32bytes".to_vec(),
        }
    }
}

#[derive(Clone)]
pub struct AmosRuntime {
    pub store: Store,
    pub memory: MemoryService,
    pub evidence: EvidenceService,
    pub scheduler: Scheduler,
    policy: PolicyEngine,
    context: ContextCompiler,
    verifier: Verifier,
    connector: Arc<dyn Connector>,
    sql_worker: SqlWorker,
    capability_issuer: CapabilityIssuer,
    statistics: StatisticsWorker,
    charts: ChartWorker,
}

impl AmosRuntime {
    pub fn open(config: RuntimeConfig) -> Result<Self> {
        let store = Store::open(&config.control_db)?;
        let policy = PolicyEngine;
        let memory = MemoryService::new(store.clone(), policy.clone());
        let scheduler = Scheduler::new(store.clone());
        let evidence = EvidenceService::new(
            store.clone(),
            memory.clone(),
            policy.clone(),
            scheduler.clone(),
        );
        let context = ContextCompiler::new(memory.clone());
        let issuer = CapabilityIssuer::new(config.capability_key)?;
        let connector = Arc::new(SqliteWarehouseConnector::new(
            TENANT,
            SOURCE,
            &config.warehouse_db,
        ));
        let sql_worker = SqlWorker::new(&config.warehouse_db, issuer.clone());
        Ok(Self {
            store,
            memory,
            evidence,
            scheduler,
            policy,
            context,
            verifier: Verifier::default(),
            connector,
            sql_worker,
            capability_issuer: issuer,
            statistics: StatisticsWorker,
            charts: ChartWorker,
        })
    }

    pub async fn run_task(
        &self,
        identity: &Identity,
        request: String,
        idempotency_key: String,
    ) -> Result<RunResult> {
        let definition = self
            .store
            .get_task_definition(&identity.tenant_id, "payment_health_review")?
            .ok_or_else(|| AmosError::NotFound("payment_health_review task definition".into()))?;
        self.policy.authorize_task(identity, &definition)?;
        let request_hash = content_hash(
            &json!({"request":request,"task":definition.task_type,"version":definition.version}),
        );
        let now = Utc::now();
        let initial = AnalyticalTransaction {
            tenant_id: identity.tenant_id.clone(),
            atxn_id: new_id("atxn"),
            request_id: new_id("req"),
            idempotency_key,
            request_hash,
            subject_id: identity.subject_id.clone(),
            request: request.clone(),
            task_type: definition.task_type.clone(),
            task_version: definition.version,
            risk_class: definition.risk_class,
            budgets: definition.budgets.clone(),
            policy_epoch: identity.policy_epoch,
            source_versions: BTreeMap::new(),
            state: AtxnState::Admitted,
            state_seq: 0,
            terminal: false,
            outcome: None,
            warnings: vec![],
            errors: vec![],
            created_at: now,
            updated_at: now,
        };
        let mut atxn = self.store.create_transaction(&initial)?;
        if atxn.atxn_id != initial.atxn_id {
            return self.load_result(&identity.tenant_id, &atxn.atxn_id);
        }
        atxn = self.advance(&atxn, AtxnState::Observing, None)?;
        let observation = self.connector.observe("table:payment_events").await?;
        atxn.source_versions.insert(
            format!("{}:payment_events", observation.source_id),
            observation.source_version.clone(),
        );
        atxn.updated_at = Utc::now();
        self.store.checkpoint_transaction(&atxn)?;
        atxn = self.advance(&atxn, AtxnState::Selecting, None)?;
        let manifest = self.context.compile(
            identity,
            &atxn.atxn_id,
            &request,
            &definition,
            try_parse_time(WINDOW_START)?,
            try_parse_time(WINDOW_END)?,
        )?;
        if !manifest.conflicts.is_empty() {
            let _ = self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?;
            return Err(AmosError::Conflict(
                "context has equal-authority conflicts".into(),
            ));
        }
        self.store.save_manifest(&manifest)?;
        atxn = self.advance(&atxn, AtxnState::Planning, None)?;
        let mut plan = self.build_plan(&atxn, &manifest);
        self.store.save_plan(&plan)?;
        atxn = self.advance(&atxn, AtxnState::Executing, None)?;
        if atxn.policy_epoch != identity.policy_epoch {
            let _ = self.advance(&atxn, AtxnState::Aborted, Some(Outcome::Abort))?;
            return Err(AmosError::Conflict(
                "policy epoch changed before execution".into(),
            ));
        }
        let mut verifications = vec![];
        let mut executions = vec![];
        for index in 0..plan.steps.len() {
            let mut repairs = 0;
            loop {
                let verification =
                    self.verifier
                        .verify_step(identity, &definition, &manifest, &plan.steps[index]);
                self.store.save_verification(&verification)?;
                verifications.push(verification.clone());
                if verification.outcome != Outcome::Repair {
                    if verification.outcome == Outcome::Reject {
                        atxn = self.advance(&atxn, AtxnState::Repairing, None)?;
                        let _ = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
                        return Err(AmosError::Validation(verification.errors.join("; ")));
                    }
                    break;
                }
                atxn = self.advance(&atxn, AtxnState::Repairing, None)?;
                if repairs >= definition.budgets.max_repairs {
                    let _ =
                        self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?;
                    return Err(AmosError::Validation("repair budget exhausted".into()));
                }
                let repair = verification
                    .permitted_repair
                    .as_deref()
                    .and_then(|repair| self.verifier.repair_step(&plan.steps[index], repair))
                    .ok_or_else(|| AmosError::Validation("permitted repair is invalid".into()))?;
                plan.steps[index] = repair;
                self.store.save_plan(&plan)?;
                atxn = self.advance(&atxn, AtxnState::Executing, None)?;
                repairs += 1;
            }
            if atxn.policy_epoch != identity.policy_epoch {
                let _ = self.advance(&atxn, AtxnState::Aborted, Some(Outcome::Abort))?;
                return Err(AmosError::Conflict(
                    "policy epoch changed before capability issue".into(),
                ));
            }
            let fence = atxn.state_seq;
            let capability =
                self.capability_issuer
                    .issue(identity, &plan, &plan.steps[index], fence)?;
            let execution =
                self.sql_worker
                    .execute(identity, &plan, &plan.steps[index], &capability, fence)?;
            self.store.save_execution(&execution)?;
            executions.push(execution);
        }
        atxn = self.advance(&atxn, AtxnState::Composing, None)?;
        let (mut artifact, mut claims, edges) = self.compose(&atxn, &manifest, &executions)?;
        atxn = self.advance(&atxn, AtxnState::Verifying, None)?;
        let step_verification_ids: Vec<String> = verifications
            .iter()
            .map(|verification| verification.verification_id.clone())
            .collect();
        for claim in &mut claims {
            claim.verification_ids = step_verification_ids.clone();
        }
        let claim_verification = self.verifier.verify_claims(
            &identity.tenant_id,
            &atxn.atxn_id,
            &definition.verifier_profile,
            &claims,
            &edges,
        );
        self.store.save_verification(&claim_verification)?;
        if claim_verification.outcome == Outcome::Reject {
            let _ = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
            return Err(AmosError::Validation(claim_verification.errors.join("; ")));
        }
        for claim in &mut claims {
            claim
                .verification_ids
                .push(claim_verification.verification_id.clone());
        }
        verifications.push(claim_verification.clone());
        atxn = self.advance(&atxn, AtxnState::Revalidating, None)?;
        let validation = self
            .connector
            .validate("table:payment_events", &observation.source_version)
            .await?;
        if !validation.same {
            let _ = self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?;
            return Err(AmosError::Conflict(
                "warehouse schema changed before commit".into(),
            ));
        }
        if atxn.policy_epoch != identity.policy_epoch {
            let _ = self.advance(&atxn, AtxnState::Aborted, Some(Outcome::Abort))?;
            return Err(AmosError::Conflict(
                "policy epoch changed before commit".into(),
            ));
        }
        let package = self.replay_package(&artifact, &manifest, &plan, &executions);
        let audit = AuditEvent {
            event_id: new_id("audit"),
            tenant_id: identity.tenant_id.clone(),
            actor_id: identity.subject_id.clone(),
            action: "evidence.commit".into(),
            target_type: "artifact".into(),
            target_id: artifact.artifact_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({"claim_count":claims.len(),"replay_level":package.replay_level}),
            created_at: Utc::now(),
        };
        atxn = self
            .store
            .commit_evidence(&atxn, &artifact, &claims, &edges, &package, &audit)?;
        let review_required = claim_verification.outcome == Outcome::NeedsReview
            || definition.publication_policy == "human_review_required";
        atxn = if review_required {
            self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?
        } else {
            atxn = self.advance(&atxn, AtxnState::ObjectFinalizing, None)?;
            artifact = self.finalize_artifact(artifact)?;
            let atxn = self.advance(&atxn, AtxnState::PublicationPending, None)?;
            self.advance(
                &atxn,
                AtxnState::Published,
                Some(if atxn.warnings.is_empty() {
                    Outcome::Pass
                } else {
                    Outcome::Warning
                }),
            )?
        };
        Ok(RunResult {
            transaction: atxn,
            manifest,
            plan,
            executions,
            verifications,
            artifact,
            claims,
            dependencies: edges,
            replay_package: package,
        })
    }

    pub fn replay(&self, identity: &Identity, artifact_id: &str) -> Result<ReplayResult> {
        let artifact = self
            .store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let package = self
            .store
            .get_replay_package(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound("replay package".into()))?;
        if package.retained_until < Utc::now() {
            return Ok(ReplayResult {
                artifact_id: artifact_id.into(),
                status: Outcome::Reject,
                matching_execution_ids: vec![],
                changed_execution_ids: vec![],
                warnings: vec![],
                errors: vec!["replay evidence expired".into()],
            });
        }
        let plan = self
            .store
            .get_plan(&identity.tenant_id, &package.plan_id)?
            .ok_or_else(|| AmosError::NotFound("replay plan".into()))?;
        let mut matching = vec![];
        let mut changed = vec![];
        for step in &plan.steps {
            let cap = self.capability_issuer.issue(identity, &plan, step, 0)?;
            let execution = self.sql_worker.execute(identity, &plan, step, &cap, 0)?;
            let expected = package.expected_execution_hashes.get(&step.step_id);
            if expected == Some(&execution.output_hash) {
                matching.push(execution.execution_id)
            } else {
                changed.push(execution.execution_id)
            }
        }
        let artifact_matches = package.expected_artifact_hash == artifact.content_hash;
        let status = if changed.is_empty() && artifact_matches {
            Outcome::Pass
        } else {
            Outcome::Warning
        };
        let mut warnings = vec![];
        if !changed.is_empty() {
            warnings.push("one or more execution hashes changed".into());
        }
        if !artifact_matches {
            warnings.push("artifact content hash differs from replay package".into());
        }
        Ok(ReplayResult {
            artifact_id: artifact.artifact_id,
            status,
            matching_execution_ids: matching,
            changed_execution_ids: changed,
            warnings,
            errors: vec![],
        })
    }

    pub fn preflight_sql(
        &self,
        identity: &Identity,
        request: &str,
        sql: String,
    ) -> Result<SqlPreflight> {
        let definition = self
            .store
            .get_task_definition(&identity.tenant_id, "payment_health_review")?
            .ok_or_else(|| AmosError::NotFound("payment_health_review task definition".into()))?;
        self.policy.authorize_task(identity, &definition)?;
        let atxn_id = new_id("preflight");
        let manifest = self.context.compile(
            identity,
            &atxn_id,
            request,
            &definition,
            try_parse_time(WINDOW_START)?,
            try_parse_time(WINDOW_END)?,
        )?;
        if !manifest.conflicts.is_empty() {
            return Err(AmosError::Conflict(
                "context has equal-authority conflicts".into(),
            ));
        }
        let proposed = step(
            "preflight",
            "preflight proposed SQL",
            sql,
            "proposed.v1",
            &manifest,
        );
        let verification = self
            .verifier
            .verify_step(identity, &definition, &manifest, &proposed);
        Ok(SqlPreflight {
            manifest_id: manifest.manifest_id,
            referenced_versions: manifest.source_versions,
            verification,
        })
    }

    pub fn revalidate_artifact(&self, identity: &Identity, artifact_id: &str) -> Result<Value> {
        self.store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let claims = self.store.list_claims(&identity.tenant_id, artifact_id)?;
        let mut changes = vec![];
        let mut updated = vec![];
        for claim in claims {
            let changed = self.revalidate_claim(identity, claim)?;
            if let Some(change) = changed.get("change").cloned() {
                changes.push(change);
            }
            if let Some(claim) = changed.get("claim").cloned() {
                updated.push(claim);
            }
        }
        self.store.append_audit(&AuditEvent {
            event_id: new_id("audit"),
            tenant_id: identity.tenant_id.clone(),
            actor_id: identity.subject_id.clone(),
            action: "artifact.revalidate".into(),
            target_type: "artifact".into(),
            target_id: artifact_id.into(),
            request_id: None,
            atxn_id: None,
            outcome: if changes.is_empty() {
                "pass".into()
            } else {
                "warning".into()
            },
            policy_epoch: identity.policy_epoch,
            details: json!({"changed_claims":changes.len()}),
            created_at: Utc::now(),
        })?;
        Ok(json!({"artifact_id":artifact_id,"changes":changes,"claims":updated}))
    }

    pub fn process_jobs(
        &self,
        identity: &Identity,
        worker: &str,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let mut results = vec![];
        for _ in 0..limit.max(1) {
            let Some(job) = self.scheduler.acquire(&identity.tenant_id, worker, 30)? else {
                break;
            };
            let fence = job.fencing_token;
            match self.dispatch_job(identity, &job) {
                Ok(detail) => {
                    self.scheduler.complete(job.clone(), fence)?;
                    results.push(json!({
                        "job_id": job.job_id,
                        "job_type": job.job_type,
                        "status": "complete",
                        "detail": detail
                    }));
                }
                Err(error) => {
                    self.scheduler.fail(job.clone(), fence, error.to_string())?;
                    results.push(json!({
                        "job_id": job.job_id,
                        "job_type": job.job_type,
                        "status": "failed",
                        "error": error.to_string()
                    }));
                }
            }
        }
        Ok(results)
    }

    pub fn drain_outbox(&self, identity: &Identity, limit: usize) -> Result<Vec<OutboxEvent>> {
        let pending = self
            .store
            .list_pending_outbox(&identity.tenant_id, limit.max(1))?;
        let mut completed = Vec::with_capacity(pending.len());
        for event in pending {
            completed.push(
                self.store
                    .complete_outbox(&identity.tenant_id, &event.event_id)?,
            );
        }
        Ok(completed)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn review_artifact(
        &self,
        identity: &Identity,
        artifact_id: &str,
        claim_ids: Vec<String>,
        decision: ReviewDecision,
        comment: String,
        correction: Option<Value>,
        authority: Authority,
    ) -> Result<ReviewResult> {
        let review: Review = self.evidence.review(
            identity,
            artifact_id,
            claim_ids,
            decision,
            comment,
            correction,
            authority,
        )?;
        let mut artifact = self
            .store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let mut claims = self.store.list_claims(&identity.tenant_id, artifact_id)?;
        let mut atxn = self
            .store
            .get_transaction(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound(artifact.atxn_id.clone()))?;

        if decision == ReviewDecision::Reject && atxn.state == AtxnState::NeedsReview {
            atxn = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
        } else if decision == ReviewDecision::Approve
            && atxn.state == AtxnState::NeedsReview
            && claims.iter().all(|claim| {
                matches!(
                    claim.review_state,
                    ReviewState::Verified | ReviewState::Approved
                )
            })
        {
            let observed = atxn
                .source_versions
                .get(&format!("{SOURCE}:payment_events"))
                .ok_or_else(|| AmosError::Validation("missing source observation".into()))?;
            let validation = self
                .connector
                .validate("table:payment_events", observed)
                .await?;
            if !validation.same || atxn.policy_epoch != identity.policy_epoch {
                return Err(AmosError::Conflict(
                    "governing state changed before reviewer publication".into(),
                ));
            }
            atxn = self.advance(&atxn, AtxnState::Revalidating, None)?;
            atxn = self.advance(&atxn, AtxnState::EvidenceCommitted, None)?;
            atxn = self.advance(&atxn, AtxnState::ObjectFinalizing, None)?;
            artifact = self.finalize_artifact(artifact)?;
            atxn = self.advance(&atxn, AtxnState::PublicationPending, None)?;
            let outcome = if self
                .store
                .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
                .iter()
                .any(|verification| verification.outcome == Outcome::Warning)
            {
                Outcome::Warning
            } else {
                Outcome::Pass
            };
            atxn = self.store.commit_local_publication(
                &atxn,
                &mut artifact,
                &mut claims,
                outcome,
                &AuditEvent {
                    event_id: new_id("audit"),
                    tenant_id: identity.tenant_id.clone(),
                    actor_id: identity.subject_id.clone(),
                    action: "artifact.publish_local".into(),
                    target_type: "artifact".into(),
                    target_id: artifact_id.into(),
                    request_id: Some(atxn.request_id.clone()),
                    atxn_id: Some(atxn.atxn_id.clone()),
                    outcome: "pass".into(),
                    policy_epoch: identity.policy_epoch,
                    details: json!({"review_id":review.review_id}),
                    created_at: Utc::now(),
                },
            )?;
        }

        Ok(ReviewResult {
            review,
            transaction: atxn,
            artifact,
            claims,
        })
    }

    pub async fn connector_health(&self) -> Result<crate::domain::ConnectorHealth> {
        self.connector.health().await
    }

    pub async fn process_source_events(
        &self,
        cursor: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<String>>> {
        let page = self.connector.subscribe(cursor).await?;
        let memory = self.store.list_active_memory(TENANT)?;
        let mut impacted = BTreeMap::new();
        for event in page.items {
            let mut claims = vec![];
            for object in memory.iter().filter(|object| {
                object.source_id == event.source_id
                    && (object.logical_key == event.subject
                        || object
                            .logical_key
                            .contains(event.subject.trim_start_matches("table:"))
                        || object
                            .content
                            .get("table")
                            .and_then(Value::as_str)
                            .is_some_and(|table| event.subject.ends_with(table)))
            }) {
                claims.extend(self.evidence.invalidate_memory(
                    &event.tenant_id,
                    &object.object_id,
                    &event.change_kind,
                )?);
            }
            impacted.insert(event.event_id, claims);
        }
        Ok(impacted)
    }

    fn advance(
        &self,
        atxn: &AnalyticalTransaction,
        next: AtxnState,
        outcome: Option<Outcome>,
    ) -> Result<AnalyticalTransaction> {
        self.store.transition_transaction(
            &atxn.tenant_id,
            &atxn.atxn_id,
            atxn.state,
            atxn.state_seq,
            next,
            outcome,
        )
    }

    fn finalize_artifact(&self, mut artifact: Artifact) -> Result<Artifact> {
        let expected = content_hash(&artifact.content);
        if expected != artifact.content_hash {
            artifact.object_state = "failed".into();
            let _ = self.store.update_artifact(&artifact);
            return Err(AmosError::Conflict(
                "artifact content hash mismatch during object finalization".into(),
            ));
        }
        artifact.object_state = "finalized".into();
        self.store.update_artifact(&artifact)?;
        Ok(artifact)
    }

    fn dispatch_job(&self, identity: &Identity, job: &Job) -> Result<Value> {
        match job.job_type.as_str() {
            "claim.revalidate" => {
                if let Some(claim_id) = job.payload.get("claim_id").and_then(Value::as_str) {
                    let claim = self
                        .store
                        .get_claim(&identity.tenant_id, claim_id)?
                        .ok_or_else(|| AmosError::NotFound(claim_id.into()))?;
                    return self.revalidate_claim(identity, claim);
                }
                let artifact_id = job
                    .payload
                    .get("artifact_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AmosError::Validation(
                            "claim.revalidate requires claim_id or artifact_id".into(),
                        )
                    })?;
                self.revalidate_artifact(identity, artifact_id)
            }
            other => Err(AmosError::Validation(format!(
                "unsupported job type: {other}"
            ))),
        }
    }

    fn revalidate_claim(&self, identity: &Identity, mut claim: Claim) -> Result<Value> {
        let before_semantic = claim.semantic_validity;
        let before_replay = claim.replay_availability;
        let before_policy = claim.policy_visibility;
        let edges = self
            .store
            .list_edges_from(&identity.tenant_id, "claim", &claim.claim_id)?;
        let mut invalid = false;
        let mut stale = false;
        let mut denied = false;
        for edge in edges
            .iter()
            .filter(|edge| edge.to.endpoint_type == "memory")
        {
            match self.store.get_memory(&identity.tenant_id, &edge.to.id)? {
                None => stale = true,
                Some(memory) => {
                    if matches!(
                        memory.status,
                        MemoryStatus::Revoked | MemoryStatus::Tombstoned
                    ) {
                        invalid = true;
                    } else if memory.status != MemoryStatus::Active
                        || memory.superseded_by.is_some()
                    {
                        stale = true;
                    }
                    if !self.policy.can_read_memory(identity, &memory) {
                        denied = true;
                    }
                }
            }
        }
        claim.semantic_validity = if invalid {
            SemanticValidity::Invalid
        } else if stale {
            SemanticValidity::Stale
        } else if matches!(before_semantic, SemanticValidity::PendingRevalidation) {
            SemanticValidity::Current
        } else {
            claim.semantic_validity
        };
        if denied {
            claim.policy_visibility = PolicyVisibility::Denied;
        }
        let replay = self
            .store
            .get_replay_package(&identity.tenant_id, &claim.artifact_id)?;
        if replay
            .as_ref()
            .is_none_or(|package| package.retained_until < Utc::now())
        {
            claim.replay_availability = ReplayAvailability::Expired;
        }
        let changed = before_semantic != claim.semantic_validity
            || before_replay != claim.replay_availability
            || before_policy != claim.policy_visibility;
        if changed {
            self.store.update_claim(&claim)?;
        }
        Ok(json!({
            "claim": claim,
            "change": if changed {
                Some(json!({
                    "claim_id": claim.claim_id,
                    "semantic_validity": {"before":before_semantic,"after":claim.semantic_validity},
                    "policy_visibility": {"before":before_policy,"after":claim.policy_visibility},
                    "replay_availability": {"before":before_replay,"after":claim.replay_availability}
                }))
            } else {
                None
            }
        }))
    }
    fn build_plan(&self, atxn: &AnalyticalTransaction, manifest: &ContextManifest) -> TypedPlan {
        let base = "event_time >= '2026-07-07T08:00:00Z' AND event_time < '2026-07-07T20:00:00Z' AND environment = 'production' AND is_test_account = 0";
        let current = "event_time >= '2026-07-07T14:00:00Z' AND event_time < '2026-07-07T20:00:00Z' AND environment = 'production' AND is_test_account = 0";
        let steps = vec![
            step(
                "summary",
                "compute current and baseline failure rates",
                format!(
                    "SELECT CASE WHEN event_time >= '{SPIKE_START}' THEN 'current' ELSE 'baseline' END AS period, SUM(CASE WHEN status='failure' THEN 1 ELSE 0 END) AS failures, COUNT(*) AS attempts, CAST(SUM(CASE WHEN status='failure' THEN 1 ELSE 0 END) AS REAL)/COUNT(*) AS failure_rate FROM payment_events WHERE {base} GROUP BY period ORDER BY period"
                ),
                "rate_comparison.v1",
                manifest,
            ),
            step(
                "concentration",
                "identify the largest processor and network contributor",
                format!(
                    "SELECT processor, card_network, SUM(CASE WHEN status='failure' THEN 1 ELSE 0 END) AS failures, COUNT(*) AS attempts, CAST(SUM(CASE WHEN status='failure' THEN 1 ELSE 0 END) AS REAL)/COUNT(*) AS failure_rate FROM payment_events WHERE {current} GROUP BY processor,card_network ORDER BY failures DESC LIMIT 10"
                ),
                "concentration.v1",
                manifest,
            ),
            step(
                "timeseries",
                "compute event-time hourly trend",
                format!(
                    "SELECT substr(event_time,1,13)||':00:00Z' AS hour, CAST(SUM(CASE WHEN status='failure' THEN 1 ELSE 0 END) AS REAL)/COUNT(*) AS failure_rate FROM payment_events WHERE {base} GROUP BY hour ORDER BY hour"
                ),
                "timeseries.v1",
                manifest,
            ),
        ];
        TypedPlan {
            plan_id: new_id("plan"),
            tenant_id: atxn.tenant_id.clone(),
            atxn_id: atxn.atxn_id.clone(),
            task_definition: manifest.task_definition.clone(),
            manifest_id: manifest.manifest_id.clone(),
            model_identity: "deterministic-alpha-planner:v1".into(),
            steps,
        }
    }

    fn compose(
        &self,
        atxn: &AnalyticalTransaction,
        manifest: &ContextManifest,
        executions: &[ExecutionRecord],
    ) -> Result<(Artifact, Vec<Claim>, Vec<DependencyEdge>)> {
        let summary = find_execution(executions, "summary")?;
        let rows = summary
            .output
            .as_array()
            .ok_or_else(|| AmosError::Execution("summary output is not rows".into()))?;
        let current = rows
            .iter()
            .find(|r| r.get("period").and_then(|v| v.as_str()) == Some("current"))
            .ok_or_else(|| AmosError::Execution("current period missing".into()))?;
        let baseline = rows
            .iter()
            .find(|r| r.get("period").and_then(|v| v.as_str()) == Some("baseline"))
            .ok_or_else(|| AmosError::Execution("baseline period missing".into()))?;
        let comparison = self.statistics.rate_comparison(
            current["failures"].as_u64().unwrap_or(0),
            current["attempts"].as_u64().unwrap_or(0),
            baseline["failures"].as_u64().unwrap_or(0),
            baseline["attempts"].as_u64().unwrap_or(0),
        )?;
        let current_rate = comparison["current_rate"].as_f64().unwrap_or(0.0);
        let baseline_rate = comparison["baseline_rate"].as_f64().unwrap_or(0.0);
        let concentration = find_execution(executions, "concentration")?;
        let top = concentration
            .output
            .as_array()
            .and_then(|r| r.first())
            .ok_or_else(|| AmosError::Execution("concentration output empty".into()))?;
        let timeseries = find_execution(executions, "timeseries")?;
        let points = timeseries
            .output
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|row| {
                Some((
                    row.get("hour")?.as_str()?.to_string(),
                    row.get("failure_rate")?.as_f64()?,
                ))
            })
            .collect::<Vec<_>>();
        let (svg, chart_hash) = self.charts.timeseries_svg(&points)?;
        let artifact_id = new_id("art");
        let report = format!(
            "# Payment failure health review\n\nFailure rate increased from {:.1}% to {:.1}%. The largest concentration was {} / {} at {:.1}%. A gateway deployment preceded the spike, but causality and the dashboard action require review.\n\nChart hash: `{}`\n\n{}",
            baseline_rate * 100.0,
            current_rate * 100.0,
            top["processor"].as_str().unwrap_or("unknown"),
            top["card_network"].as_str().unwrap_or("unknown"),
            top["failure_rate"].as_f64().unwrap_or(0.0) * 100.0,
            chart_hash,
            svg
        );
        let artifact = Artifact {
            tenant_id: atxn.tenant_id.clone(),
            artifact_id: artifact_id.clone(),
            atxn_id: atxn.atxn_id.clone(),
            artifact_type: "report".into(),
            title: "Payment failure health review".into(),
            content: report.clone(),
            content_hash: content_hash(&report),
            audience: "internal".into(),
            risk_class: RiskClass::MaterialInternal,
            object_state: "pending_promotion".into(),
            publication_validity: PublicationValidity::Draft,
            created_at: Utc::now(),
        };
        let claims=vec![claim(&artifact,"metric_comparison",format!("Payment failure rate increased from {:.1}% to {:.1}%.",baseline_rate*100.0,current_rate*100.0),json!({"metric_id":"payment_failure_rate:v3","current_value":current_rate,"baseline_value":baseline_rate}),vec![summary.execution_id.clone()]),claim(&artifact,"concentration",format!("The largest failure concentration was {} / {}.",top["processor"].as_str().unwrap_or("unknown"),top["card_network"].as_str().unwrap_or("unknown")),top.clone(),vec![concentration.execution_id.clone()]),claim(&artifact,"causal","The gateway deployment may have contributed to the spike.".into(),json!({"review_required":true}),vec![]),claim(&artifact,"operational_recommendation","Update the executive dashboard with a warning while the cause remains under review.".into(),json!({"review_required":true}),vec![])];
        let metric = role_id(manifest, "metric_definition")?;
        let schema = role_id(manifest, "active_schema")?;
        let data = role_id(manifest, "data_snapshot")?;
        let mut edges = vec![];
        for item in claims
            .iter()
            .filter(|c| matches!(c.claim_type.as_str(), "metric_comparison" | "concentration"))
        {
            for execution in &item.support_execution_ids {
                edges.push(edge(
                    atxn,
                    &item.claim_id,
                    "computed_by",
                    "execution",
                    execution,
                    None,
                ));
            }
            edges.push(edge(
                atxn,
                &item.claim_id,
                "governed_by_metric",
                "memory",
                &metric,
                None,
            ));
            edges.push(edge(
                atxn,
                &item.claim_id,
                "governed_by_schema",
                "memory",
                &schema,
                None,
            ));
            edges.push(edge(
                atxn,
                &item.claim_id,
                "scoped_to_data_state",
                "memory",
                &data,
                None,
            ));
        }
        if let Some(document) = manifest.optional_selected.iter().find(|id| {
            manifest.selected_objects.iter().any(|o| {
                &o.object_id == *id && matches!(o.memory_type, crate::domain::MemoryType::Document)
            })
        }) {
            for item in claims.iter().filter(|c| {
                matches!(
                    c.claim_type.as_str(),
                    "causal" | "operational_recommendation"
                )
            }) {
                edges.push(edge(
                    atxn,
                    &item.claim_id,
                    "supported_by_document",
                    "memory",
                    document,
                    None,
                ));
            }
        }
        Ok((artifact, claims, edges))
    }

    fn replay_package(
        &self,
        artifact: &Artifact,
        manifest: &ContextManifest,
        plan: &TypedPlan,
        executions: &[ExecutionRecord],
    ) -> ReplayPackage {
        ReplayPackage {
            package_id: new_id("rpl"),
            tenant_id: artifact.tenant_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            replay_level: 3,
            manifest_id: manifest.manifest_id.clone(),
            plan_id: plan.plan_id.clone(),
            execution_ids: executions.iter().map(|e| e.execution_id.clone()).collect(),
            template: "payment_health_report:v3".into(),
            render_config_hash: content_hash(&"payment_health_report:v3"),
            retained_until: Utc::now() + Duration::days(365),
            expected_artifact_hash: artifact.content_hash.clone(),
            expected_execution_hashes: executions
                .iter()
                .map(|e| (e.step_id.clone(), e.output_hash.clone()))
                .collect(),
            source_versions: manifest.source_versions.clone(),
        }
    }
    fn load_result(&self, tenant: &str, atxn_id: &str) -> Result<RunResult> {
        let transaction = self
            .store
            .get_transaction(tenant, atxn_id)?
            .ok_or_else(|| AmosError::NotFound(atxn_id.into()))?;
        let artifact = self
            .store
            .get_artifact_by_atxn(tenant, atxn_id)?
            .ok_or_else(|| {
                AmosError::Conflict("idempotent transaction is still in progress".into())
            })?;
        let package = self
            .store
            .get_replay_package(tenant, &artifact.artifact_id)?
            .ok_or_else(|| AmosError::NotFound("replay package".into()))?;
        let manifest = self
            .store
            .get_manifest(tenant, &package.manifest_id)?
            .ok_or_else(|| AmosError::NotFound("context manifest".into()))?;
        let plan = self
            .store
            .get_plan(tenant, &package.plan_id)?
            .ok_or_else(|| AmosError::NotFound("plan".into()))?;
        let claims = self.store.list_claims(tenant, &artifact.artifact_id)?;
        let mut dependencies = Vec::new();
        for claim in &claims {
            dependencies.extend(
                self.store
                    .list_edges_from(tenant, "claim", &claim.claim_id)?,
            );
        }
        Ok(RunResult {
            transaction,
            manifest,
            plan,
            executions: self.store.list_executions(tenant, atxn_id)?,
            verifications: self.store.list_verifications(tenant, atxn_id)?,
            artifact,
            claims,
            dependencies,
            replay_package: package,
        })
    }
}

fn step(
    id: &str,
    purpose: &str,
    sql: String,
    output: &str,
    manifest: &ContextManifest,
) -> PlanStep {
    PlanStep {
        step_id: id.into(),
        purpose: purpose.into(),
        tool: "sql.readonly.v1".into(),
        source_id: SOURCE.into(),
        input_object_ids: manifest
            .selected_objects
            .iter()
            .map(|o| o.object_id.clone())
            .collect(),
        parameter_schema: format!("{id}.v1"),
        parameters: json!({"sql":sql,"relations":["analytics","payments"]}),
        expected_output_schema: output.into(),
        limits: crate::domain::OperationLimits {
            seconds: 30,
            rows: 50_000,
            bytes: 5_000_000,
        },
        max_attempts: 2,
        repair_classes: BTreeSet::from(["COLUMN_SUPERSEDED".into()]),
        verifier_profile: "payment_health.v2".into(),
    }
}
fn claim(
    artifact: &Artifact,
    claim_type: &str,
    text: String,
    payload: Value,
    executions: Vec<String>,
) -> Claim {
    Claim {
        tenant_id: artifact.tenant_id.clone(),
        claim_id: new_id("clm"),
        artifact_id: artifact.artifact_id.clone(),
        claim_type: claim_type.into(),
        text,
        payload,
        risk_class: artifact.risk_class,
        support_execution_ids: executions,
        verification_ids: vec![],
        publication_validity: PublicationValidity::Draft,
        semantic_validity: SemanticValidity::Current,
        policy_visibility: PolicyVisibility::Allowed,
        replay_availability: ReplayAvailability::Level3,
        review_state: if matches!(claim_type, "causal" | "operational_recommendation") {
            ReviewState::NeedsReview
        } else {
            ReviewState::Verified
        },
        supersession_state: SupersessionState::Active,
    }
}
fn edge(
    atxn: &AnalyticalTransaction,
    claim: &str,
    relation: &str,
    target_type: &str,
    target: &str,
    version: Option<String>,
) -> DependencyEdge {
    let mut e = DependencyEdge {
        edge_id: new_id("edge"),
        tenant_id: atxn.tenant_id.clone(),
        from: EdgeEndpoint {
            endpoint_type: "claim".into(),
            id: claim.into(),
        },
        relation: relation.into(),
        to: EdgeEndpoint {
            endpoint_type: target_type.into(),
            id: target.into(),
        },
        source_version: version,
        created_by_atxn: atxn.atxn_id.clone(),
        content_hash: String::new(),
    };
    e.content_hash = content_hash(&e);
    e
}
fn role_id(manifest: &ContextManifest, role: &str) -> Result<String> {
    manifest
        .required_role_coverage
        .get(role)
        .and_then(|ids| ids.first())
        .cloned()
        .ok_or_else(|| AmosError::RequiredRoleMissing(role.into()))
}
fn find_execution<'a>(values: &'a [ExecutionRecord], step: &str) -> Result<&'a ExecutionRecord> {
    values
        .iter()
        .find(|e| e.step_id == step)
        .ok_or_else(|| AmosError::NotFound(format!("execution {step}")))
}
fn try_parse_time(value: &str) -> Result<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AmosError::Validation(format!("invalid timestamp '{value}': {error}")))
}
