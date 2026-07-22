use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use chrono::{Duration, Utc};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::{
    Result,
    connectors::{Connector, Page, SqliteWarehouseConnector},
    context::ContextCompiler,
    domain::{
        AnalyticalTransaction, Artifact, AtxnState, AuditEvent, Authority, Claim, ContextManifest,
        DependencyEdge, EdgeEndpoint, ErasureReceipt, ExecutionRecord, Identity, Job, Outcome,
        PlanStep, PolicyVisibility, PublicationValidity, ReplayAvailability, ReplayComparisonKind,
        ReplayExecutionComparison, ReplayPackage, ReplayResult, RetentionCommand, RetentionRecord,
        Review, ReviewDecision, ReviewResult, ReviewState, RiskClass, RunResult, SemanticValidity,
        SqlPreflight, SupersessionState, TaskDefinition, TypedPlan, VerificationRecord,
        content_hash, new_id, stable_id,
    },
    error::AmosError,
    evidence::EvidenceService,
    memory::MemoryService,
    observability::{MetricsSnapshot, OperationalMetrics},
    policy::PolicyEngine,
    publication::{LocalFilesystemObjectStore, ObjectStore},
    scheduler::Scheduler,
    seed::{SOURCE, SPIKE_START, TENANT, WINDOW_END, WINDOW_START},
    store::Store,
    verification::{ClaimVerificationRequest, Verifier},
    workers::{CapabilityIssuer, ChartWorker, SqlWorker, StatisticsWorker},
};

#[derive(Clone)]
pub struct RuntimeConfig {
    pub control_db: PathBuf,
    pub warehouse_db: PathBuf,
    pub object_root: PathBuf,
    capability_key: Vec<u8>,
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("control_db", &self.control_db)
            .field("warehouse_db", &self.warehouse_db)
            .field("object_root", &self.object_root)
            .field("capability_key", &"[REDACTED]")
            .finish()
    }
}

impl RuntimeConfig {
    pub fn new(
        control_db: impl Into<PathBuf>,
        warehouse_db: impl Into<PathBuf>,
        capability_key: impl Into<Vec<u8>>,
    ) -> Self {
        let control_db = control_db.into();
        let warehouse_db = warehouse_db.into();
        let object_root = control_db
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("objects");
        Self {
            control_db,
            warehouse_db,
            object_root,
            capability_key: capability_key.into(),
        }
    }

    pub fn demo(root: impl AsRef<Path>) -> Self {
        Self::new(
            root.as_ref().join("data/amos.sqlite"),
            root.as_ref().join("data/payments.sqlite"),
            b"amos-explicit-demo-capability-key-v1".to_vec(),
        )
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
    blocking_permits: Arc<Semaphore>,
    metrics: Arc<OperationalMetrics>,
    object_store: LocalFilesystemObjectStore,
}

struct PreparedEvidence {
    artifact: Artifact,
    claims: Vec<Claim>,
    edges: Vec<DependencyEdge>,
    executions: Vec<ExecutionRecord>,
    claim_verification: VerificationRecord,
}

enum ResumeOutcome {
    Completed(Box<RunResult>),
    Paused(Box<AnalyticalTransaction>),
}

impl ResumeOutcome {
    fn into_result(self) -> Result<RunResult> {
        match self {
            Self::Completed(result) => Ok(*result),
            Self::Paused(atxn) => Err(AmosError::Conflict(format!(
                "transaction {} paused at {:?}",
                atxn.atxn_id, atxn.state
            ))),
        }
    }
}

impl AmosRuntime {
    pub fn open(config: RuntimeConfig) -> Result<Self> {
        let store = Store::open(&config.control_db)?;
        let policy = PolicyEngine;
        let memory = MemoryService::new(store.clone(), policy.clone());
        let scheduler = Scheduler::new(store.clone());
        let evidence = EvidenceService::new(store.clone(), policy.clone());
        let context = ContextCompiler::new(memory.clone());
        let issuer = CapabilityIssuer::new(config.capability_key)?;
        let connector = Arc::new(SqliteWarehouseConnector::new(
            TENANT,
            SOURCE,
            &config.warehouse_db,
            issuer.clone(),
        ));
        let sql_worker = SqlWorker::new(&config.warehouse_db, issuer.clone());
        let object_store = LocalFilesystemObjectStore::new(&config.object_root)?;
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
            blocking_permits: Arc::new(Semaphore::new(8)),
            metrics: Arc::new(OperationalMetrics::default()),
            object_store,
        })
    }

    pub fn metrics(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    pub(crate) async fn execute_blocking<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&AmosRuntime) -> Result<T> + Send + 'static,
    {
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(&runtime)
        })
        .await
        .map_err(|error| AmosError::Storage(format!("blocking operation join failed: {error}")))?
    }

    pub fn get_transaction_for(
        &self,
        identity: &Identity,
        atxn_id: &str,
    ) -> Result<AnalyticalTransaction> {
        let transaction = self
            .store
            .get_transaction(&identity.tenant_id, atxn_id)?
            .ok_or_else(|| AmosError::NotFound(atxn_id.into()))?;
        self.policy
            .authorize_transaction_read(identity, &transaction)?;
        Ok(transaction)
    }

    pub fn list_artifacts_for(&self, identity: &Identity, limit: usize) -> Result<Vec<Artifact>> {
        let mut visible = Vec::new();
        for artifact in self.store.list_artifacts(&identity.tenant_id, limit)? {
            let transaction = self.artifact_transaction(identity, &artifact)?;
            let artifact_allowed = self
                .policy
                .authorize_artifact_read(identity, &artifact, &transaction)
                .is_ok();
            let claims = self
                .store
                .list_claims(&identity.tenant_id, &artifact.artifact_id)?;
            let claims_allowed = claims.iter().all(|claim| {
                self.policy
                    .authorize_claim_read(identity, &transaction, claim)
                    .is_ok()
            });
            if artifact_allowed && claims_allowed {
                visible.push(artifact);
            }
        }
        Ok(visible)
    }

    pub fn list_artifacts_page_for(
        &self,
        identity: &Identity,
        after_artifact_id: Option<&str>,
        limit: usize,
    ) -> Result<Page<Artifact>> {
        if limit == 0 || limit > 100 {
            return Err(AmosError::Validation(
                "artifact page limit must be between 1 and 100".into(),
            ));
        }
        let mut visible = Vec::with_capacity(limit);
        let mut cursor = after_artifact_id.map(str::to_string);
        let mut scanned = 0_usize;
        let mut more = false;
        while visible.len() < limit && scanned < 1_000 {
            let batch =
                self.store
                    .list_artifacts_after(&identity.tenant_id, cursor.as_deref(), 100)?;
            if batch.is_empty() {
                break;
            }
            more = batch.len() == 100;
            for artifact in batch {
                scanned += 1;
                cursor = Some(artifact.artifact_id.clone());
                let transaction = self.artifact_transaction(identity, &artifact)?;
                let claims = self
                    .store
                    .list_claims(&identity.tenant_id, &artifact.artifact_id)?;
                if self
                    .policy
                    .authorize_artifact_read(identity, &artifact, &transaction)
                    .is_ok()
                    && claims.iter().all(|claim| {
                        self.policy
                            .authorize_claim_read(identity, &transaction, claim)
                            .is_ok()
                    })
                {
                    visible.push(artifact);
                    if visible.len() == limit {
                        more = true;
                        break;
                    }
                }
            }
            if !more {
                break;
            }
        }
        Ok(Page {
            items: visible,
            next_cursor: more.then_some(cursor).flatten(),
        })
    }

    pub fn get_artifact_for(
        &self,
        identity: &Identity,
        artifact_id: &str,
    ) -> Result<(Artifact, Vec<Claim>, Vec<DependencyEdge>)> {
        let artifact = self
            .store
            .get_artifact(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound(artifact_id.into()))?;
        let transaction = self.artifact_transaction(identity, &artifact)?;
        self.policy
            .authorize_artifact_read(identity, &artifact, &transaction)?;
        let claims = self.store.list_claims(&identity.tenant_id, artifact_id)?;
        let mut edges = Vec::new();
        for claim in &claims {
            self.policy
                .authorize_claim_read(identity, &transaction, claim)?;
            edges.extend(self.store.list_edges_from(
                &identity.tenant_id,
                "claim",
                &claim.claim_id,
            )?);
        }
        Ok((artifact, claims, edges))
    }

    pub fn get_claim_for(
        &self,
        identity: &Identity,
        claim_id: &str,
    ) -> Result<(Claim, Vec<DependencyEdge>)> {
        let claim = self
            .store
            .get_claim(&identity.tenant_id, claim_id)?
            .ok_or_else(|| AmosError::NotFound(claim_id.into()))?;
        let artifact = self
            .store
            .get_artifact(&identity.tenant_id, &claim.artifact_id)?
            .ok_or_else(|| AmosError::NotFound(claim.artifact_id.clone()))?;
        let transaction = self.artifact_transaction(identity, &artifact)?;
        self.policy
            .authorize_artifact_read(identity, &artifact, &transaction)?;
        self.policy
            .authorize_claim_read(identity, &transaction, &claim)?;
        let dependencies = self
            .store
            .list_edges_from(&identity.tenant_id, "claim", claim_id)?;
        Ok((claim, dependencies))
    }

    pub fn authorize_operations(&self, identity: &Identity) -> Result<()> {
        self.policy.authorize_operations(identity)
    }

    pub fn authorize_review_queue(&self, identity: &Identity) -> Result<()> {
        self.policy.authorize_review(identity, false)
    }

    pub fn set_retention(
        &self,
        identity: &Identity,
        command: RetentionCommand,
    ) -> Result<RetentionRecord> {
        self.policy.authorize_operations(identity)?;
        self.store.set_retention(
            &RetentionRecord {
                tenant_id: identity.tenant_id.clone(),
                target_type: command.target_type,
                target_id: command.target_id,
                retained_until: command.retained_until,
                legal_hold: command.legal_hold,
                reason: command.reason,
                updated_by: identity.subject_id.clone(),
                updated_at: Utc::now(),
            },
            &command.idempotency_key,
        )
    }

    pub fn erase_memory(
        &self,
        identity: &Identity,
        object_id: &str,
        idempotency_key: &str,
    ) -> Result<ErasureReceipt> {
        self.policy.authorize_operations(identity)?;
        self.store.erase_memory(
            &identity.tenant_id,
            object_id,
            &identity.subject_id,
            idempotency_key,
            Utc::now(),
        )
    }

    pub async fn run_task(
        &self,
        identity: &Identity,
        request: String,
        idempotency_key: String,
    ) -> Result<RunResult> {
        self.metrics.task_started();
        let started = Instant::now();
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        let identity = identity.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AmosError::Storage(error.to_string()))?
                .block_on(runtime.run_task_inner(&identity, request, idempotency_key))
        })
        .await
        .map_err(|error| AmosError::Storage(format!("task worker join failed: {error}")))?;
        self.metrics.task_finished(
            result.is_ok(),
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        result
    }

    async fn run_task_inner(
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
        )?;
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
            self.policy.authorize_transaction_read(identity, &atxn)?;
            return match atxn.state {
                AtxnState::Published | AtxnState::NeedsReview => {
                    self.load_result(&identity.tenant_id, &atxn.atxn_id)
                }
                AtxnState::Rejected | AtxnState::Aborted | AtxnState::Revoked => Err(
                    AmosError::Conflict("idempotent transaction ended without evidence".into()),
                ),
                _ => self
                    .resume_task_inner(identity, atxn, None)
                    .await?
                    .into_result(),
            };
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
            parse_time(WINDOW_START)?,
            parse_time(WINDOW_END)?,
        )?;
        if !manifest.conflicts.is_empty() {
            let _ = self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?;
            return Err(AmosError::Conflict(
                "context has equal-authority conflicts".into(),
            ));
        }
        self.store.save_manifest(&manifest)?;
        atxn = self.advance(&atxn, AtxnState::Planning, None)?;
        let mut plan = self.build_plan(&atxn, &manifest)?;
        let mut verifications = vec![];
        for index in 0..plan.steps.len() {
            let mut repairs = 0;
            loop {
                let verification = self.verifier.verify_step(
                    identity,
                    &definition,
                    &manifest,
                    &plan.steps[index],
                )?;
                self.store.save_verification(&verification)?;
                verifications.push(verification.clone());
                if verification.outcome != Outcome::Repair {
                    if verification.outcome == Outcome::Reject {
                        let _ = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
                        return Err(AmosError::Validation(verification.errors.join("; ")));
                    }
                    break;
                }
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
                repairs += 1;
            }
        }
        self.store.save_plan(&plan)?;
        atxn = self.advance(&atxn, AtxnState::Executing, None)?;
        let fence = atxn.state_seq;
        let mut executions = vec![];
        for step in &plan.steps {
            let capability = self.capability_issuer.issue(identity, &plan, step, fence)?;
            let execution = self
                .sql_worker
                .execute(identity, &plan, step, &capability, fence)?;
            let execution = self.store.save_execution(&execution)?;
            executions.push(execution);
        }
        atxn = self.advance(&atxn, AtxnState::Composing, None)?;
        let (artifact, mut claims, edges) = self.compose(&atxn, &manifest, &executions)?;
        for claim in claims.iter_mut().filter(|claim| {
            matches!(
                claim.claim_type.as_str(),
                "metric_value" | "metric_comparison" | "concentration"
            )
        }) {
            for execution_id in &claim.support_execution_ids {
                let execution = executions
                    .iter()
                    .find(|execution| &execution.execution_id == execution_id)
                    .ok_or_else(|| AmosError::NotFound(execution_id.clone()))?;
                let step = plan
                    .steps
                    .iter()
                    .find(|step| step.step_id == execution.step_id)
                    .ok_or_else(|| AmosError::NotFound(execution.step_id.clone()))?;
                let step_hash = content_hash(step)?;
                claim.verification_ids.extend(
                    verifications
                        .iter()
                        .filter(|verification| verification.input_hash == step_hash)
                        .map(|verification| verification.verification_id.clone()),
                );
            }
            claim.verification_ids.sort();
            claim.verification_ids.dedup();
        }
        atxn = self.advance(&atxn, AtxnState::Verifying, None)?;
        let claim_verification = self.verifier.verify_claims(&ClaimVerificationRequest {
            tenant: &identity.tenant_id,
            atxn_id: &atxn.atxn_id,
            profile: &definition.verifier_profile,
            artifact: &artifact,
            manifest: &manifest,
            claims: &claims,
            edges: &edges,
            executions: &executions,
            verifications: &verifications,
        })?;
        self.store.save_verification(&claim_verification)?;
        if claim_verification.outcome == Outcome::Reject {
            let _ = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
            return Err(AmosError::Validation(claim_verification.errors.join("; ")));
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
            return Err(AmosError::Conflict(
                "policy epoch changed before commit".into(),
            ));
        }
        let package = self.replay_package(&artifact, &manifest, &plan, &executions)?;
        let audit = AuditEvent {
            event_id: stable_id("audit", &(&atxn.atxn_id, "evidence.commit"))?,
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
            created_at: atxn.created_at,
        };
        atxn = self
            .store
            .commit_evidence(&atxn, &artifact, &claims, &edges, &package, &audit)?;
        let review_required = claim_verification.outcome == Outcome::NeedsReview
            || definition.publication_policy == "human_review_required";
        atxn = if review_required {
            self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?
        } else {
            let atxn = self.advance(&atxn, AtxnState::ObjectFinalizing, None)?;
            self.finalize_object(&atxn)?;
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

    pub async fn recover_task(&self, identity: &Identity, atxn_id: String) -> Result<RunResult> {
        self.metrics.recovery_started();
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        let identity = identity.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let atxn = runtime
                .store
                .get_transaction(&identity.tenant_id, &atxn_id)?
                .ok_or_else(|| AmosError::NotFound(atxn_id.clone()))?;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AmosError::Storage(error.to_string()))?
                .block_on(runtime.resume_task_inner(&identity, atxn, None))?
                .into_result()
        })
        .await
        .map_err(|error| AmosError::Storage(format!("recovery worker join failed: {error}")))?;
        self.metrics.recovery_finished(result.is_ok());
        result
    }

    pub async fn recover_task_until_checkpoint(
        &self,
        identity: &Identity,
        atxn_id: String,
        stop_before: AtxnState,
    ) -> Result<AnalyticalTransaction> {
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        let identity = identity.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let atxn = runtime
                .store
                .get_transaction(&identity.tenant_id, &atxn_id)?
                .ok_or_else(|| AmosError::NotFound(atxn_id.clone()))?;
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AmosError::Storage(error.to_string()))?
                .block_on(runtime.resume_task_inner(&identity, atxn, Some(stop_before)))?;
            Ok(match outcome {
                ResumeOutcome::Completed(result) => result.transaction,
                ResumeOutcome::Paused(atxn) => *atxn,
            })
        })
        .await
        .map_err(|error| {
            AmosError::Storage(format!("checkpoint recovery worker join failed: {error}"))
        })?
    }

    async fn resume_task_inner(
        &self,
        identity: &Identity,
        mut atxn: AnalyticalTransaction,
        stop_before: Option<AtxnState>,
    ) -> Result<ResumeOutcome> {
        if atxn.tenant_id != identity.tenant_id || atxn.subject_id != identity.subject_id {
            return Err(AmosError::PermissionDenied(
                "only the admitting subject may resume an analytical transaction".into(),
            ));
        }
        if atxn.policy_epoch != identity.policy_epoch {
            return Err(AmosError::Conflict(
                "policy epoch changed before transaction recovery".into(),
            ));
        }
        let definition = self
            .store
            .get_task_definition_version(&identity.tenant_id, &atxn.task_type, atxn.task_version)?
            .ok_or_else(|| {
                AmosError::NotFound(format!(
                    "{} task definition version {}",
                    atxn.task_type, atxn.task_version
                ))
            })?;
        self.policy.authorize_task(identity, &definition)?;
        let mut prepared = None;
        loop {
            if stop_before == Some(atxn.state) {
                return Ok(ResumeOutcome::Paused(Box::new(atxn)));
            }
            match atxn.state {
                AtxnState::Admitted => {
                    atxn = self.advance(&atxn, AtxnState::Observing, None)?;
                }
                AtxnState::Observing => {
                    if !atxn
                        .source_versions
                        .contains_key(&format!("{SOURCE}:payment_events"))
                    {
                        let observation = self.connector.observe("table:payment_events").await?;
                        atxn.source_versions.insert(
                            format!("{}:payment_events", observation.source_id),
                            observation.source_version,
                        );
                        atxn.updated_at = Utc::now();
                        self.store.checkpoint_transaction(&atxn)?;
                    }
                    atxn = self.advance(&atxn, AtxnState::Selecting, None)?;
                }
                AtxnState::Selecting => {
                    let manifest = match self
                        .store
                        .get_manifest_by_atxn(&identity.tenant_id, &atxn.atxn_id)?
                    {
                        Some(manifest) => manifest,
                        None => {
                            let manifest = self.context.compile(
                                identity,
                                &atxn.atxn_id,
                                &atxn.request,
                                &definition,
                                parse_time(WINDOW_START)?,
                                parse_time(WINDOW_END)?,
                            )?;
                            if !manifest.conflicts.is_empty() {
                                let _ = self.advance(
                                    &atxn,
                                    AtxnState::NeedsReview,
                                    Some(Outcome::NeedsReview),
                                )?;
                                return Err(AmosError::Conflict(
                                    "context has equal-authority conflicts".into(),
                                ));
                            }
                            self.store.save_manifest(&manifest)?;
                            manifest
                        }
                    };
                    if manifest.tenant_id != atxn.tenant_id
                        || manifest.atxn_id != atxn.atxn_id
                        || manifest.policy_epoch != atxn.policy_epoch
                    {
                        return Err(AmosError::Conflict(
                            "persisted manifest does not match the recovery checkpoint".into(),
                        ));
                    }
                    atxn = self.advance(&atxn, AtxnState::Planning, None)?;
                }
                AtxnState::Planning => {
                    let manifest = self.recovery_manifest(&atxn)?;
                    if self
                        .store
                        .get_plan_by_atxn(&identity.tenant_id, &atxn.atxn_id)?
                        .is_none()
                    {
                        let mut plan = self.build_plan(&atxn, &manifest)?;
                        for index in 0..plan.steps.len() {
                            let mut repairs = 0;
                            loop {
                                let verification = self.verifier.verify_step(
                                    identity,
                                    &definition,
                                    &manifest,
                                    &plan.steps[index],
                                )?;
                                self.store.save_verification(&verification)?;
                                if verification.outcome != Outcome::Repair {
                                    if verification.outcome == Outcome::Reject {
                                        let _ = self.advance(
                                            &atxn,
                                            AtxnState::Rejected,
                                            Some(Outcome::Reject),
                                        )?;
                                        return Err(AmosError::Validation(
                                            verification.errors.join("; "),
                                        ));
                                    }
                                    break;
                                }
                                if repairs >= definition.budgets.max_repairs {
                                    let _ = self.advance(
                                        &atxn,
                                        AtxnState::NeedsReview,
                                        Some(Outcome::NeedsReview),
                                    )?;
                                    return Err(AmosError::Validation(
                                        "repair budget exhausted".into(),
                                    ));
                                }
                                plan.steps[index] = verification
                                    .permitted_repair
                                    .as_deref()
                                    .and_then(|repair| {
                                        self.verifier.repair_step(&plan.steps[index], repair)
                                    })
                                    .ok_or_else(|| {
                                        AmosError::Validation("permitted repair is invalid".into())
                                    })?;
                                repairs += 1;
                            }
                        }
                        self.store.save_plan(&plan)?;
                    }
                    atxn = self.advance(&atxn, AtxnState::Executing, None)?;
                }
                AtxnState::Repairing => {
                    atxn = self.advance(&atxn, AtxnState::Executing, None)?;
                }
                AtxnState::Executing => {
                    let plan = self.recovery_plan(&atxn)?;
                    let persisted = self
                        .store
                        .list_executions(&identity.tenant_id, &atxn.atxn_id)?;
                    for step in &plan.steps {
                        if persisted.iter().any(|execution| {
                            execution.step_id == step.step_id
                                && execution.fencing_token == atxn.state_seq
                        }) {
                            continue;
                        }
                        let capability =
                            self.capability_issuer
                                .issue(identity, &plan, step, atxn.state_seq)?;
                        let execution = self.sql_worker.execute(
                            identity,
                            &plan,
                            step,
                            &capability,
                            atxn.state_seq,
                        )?;
                        self.store.save_execution(&execution)?;
                    }
                    atxn = self.advance(&atxn, AtxnState::Composing, None)?;
                }
                AtxnState::Composing => {
                    let manifest = self.recovery_manifest(&atxn)?;
                    let executions = self.recovery_executions(&identity.tenant_id, &atxn)?;
                    self.compose(&atxn, &manifest, &executions)?;
                    atxn = self.advance(&atxn, AtxnState::Verifying, None)?;
                }
                AtxnState::Verifying => {
                    let candidate = self.prepare_evidence(identity, &atxn, &definition)?;
                    if candidate.claim_verification.outcome == Outcome::Reject {
                        let _ = self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
                        return Err(AmosError::Validation(
                            candidate.claim_verification.errors.join("; "),
                        ));
                    }
                    atxn = self.advance(&atxn, AtxnState::Revalidating, None)?;
                    prepared = Some(candidate);
                }
                AtxnState::Revalidating => {
                    if self
                        .store
                        .get_artifact_by_atxn(&identity.tenant_id, &atxn.atxn_id)?
                        .is_some()
                    {
                        let observed = atxn
                            .source_versions
                            .get(&format!("{SOURCE}:payment_events"))
                            .ok_or_else(|| {
                                AmosError::Validation(
                                    "review recovery has no source observation".into(),
                                )
                            })?;
                        let validation = self
                            .connector
                            .validate("table:payment_events", observed)
                            .await?;
                        if !validation.same || atxn.policy_epoch != identity.policy_epoch {
                            return Err(AmosError::Conflict(
                                "governing state changed before review recovery".into(),
                            ));
                        }
                        atxn = self.advance(&atxn, AtxnState::EvidenceCommitted, None)?;
                        continue;
                    }
                    let candidate = match prepared.take() {
                        Some(candidate) => candidate,
                        None => self.prepare_evidence(identity, &atxn, &definition)?,
                    };
                    let observed = atxn
                        .source_versions
                        .get(&format!("{SOURCE}:payment_events"))
                        .ok_or_else(|| {
                            AmosError::Validation(
                                "recovery checkpoint has no source observation".into(),
                            )
                        })?;
                    let validation = self
                        .connector
                        .validate("table:payment_events", observed)
                        .await?;
                    if !validation.same {
                        let _ = self.advance(
                            &atxn,
                            AtxnState::NeedsReview,
                            Some(Outcome::NeedsReview),
                        )?;
                        return Err(AmosError::Conflict(
                            "warehouse schema changed before recovery commit".into(),
                        ));
                    }
                    if atxn.policy_epoch != identity.policy_epoch {
                        return Err(AmosError::Conflict(
                            "policy epoch changed before recovery commit".into(),
                        ));
                    }
                    let package = self.replay_package(
                        &candidate.artifact,
                        &self.recovery_manifest(&atxn)?,
                        &self.recovery_plan(&atxn)?,
                        &candidate.executions,
                    )?;
                    atxn = self.store.commit_evidence(
                        &atxn,
                        &candidate.artifact,
                        &candidate.claims,
                        &candidate.edges,
                        &package,
                        &AuditEvent {
                            event_id: stable_id("audit", &(&atxn.atxn_id, "evidence.commit"))?,
                            tenant_id: identity.tenant_id.clone(),
                            actor_id: identity.subject_id.clone(),
                            action: "evidence.commit".into(),
                            target_type: "artifact".into(),
                            target_id: candidate.artifact.artifact_id.clone(),
                            request_id: Some(atxn.request_id.clone()),
                            atxn_id: Some(atxn.atxn_id.clone()),
                            outcome: "pass".into(),
                            policy_epoch: identity.policy_epoch,
                            details: json!({
                                "claim_count": candidate.claims.len(),
                                "replay_level": package.replay_level,
                                "recovered": true,
                            }),
                            created_at: atxn.created_at,
                        },
                    )?;
                }
                AtxnState::EvidenceCommitted => {
                    let artifact = self
                        .store
                        .get_artifact_by_atxn(&identity.tenant_id, &atxn.atxn_id)?
                        .ok_or_else(|| AmosError::NotFound("committed artifact".into()))?;
                    let claims = self
                        .store
                        .list_claims(&identity.tenant_id, &artifact.artifact_id)?;
                    let obligations_satisfied = !claims.is_empty()
                        && claims.iter().all(|claim| {
                            matches!(
                                claim.review_state,
                                ReviewState::Verified | ReviewState::Approved
                            )
                        });
                    let requires_review = !obligations_satisfied
                        && (definition.publication_policy == "human_review_required"
                            || self
                                .store
                                .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
                                .iter()
                                .any(|verification| {
                                    verification.profile_version >= 2
                                        && verification.outcome == Outcome::NeedsReview
                                }));
                    atxn = if requires_review {
                        self.advance(&atxn, AtxnState::NeedsReview, Some(Outcome::NeedsReview))?
                    } else {
                        self.advance(&atxn, AtxnState::ObjectFinalizing, None)?
                    };
                }
                AtxnState::ObjectFinalizing => {
                    self.finalize_object(&atxn)?;
                    atxn = self.advance(&atxn, AtxnState::PublicationPending, None)?;
                }
                AtxnState::PublicationPending => {
                    let mut result = self.load_result(&identity.tenant_id, &atxn.atxn_id)?;
                    let outcome = if result
                        .verifications
                        .iter()
                        .any(|verification| verification.outcome == Outcome::Warning)
                    {
                        Outcome::Warning
                    } else {
                        Outcome::Pass
                    };
                    let artifact_id = result.artifact.artifact_id.clone();
                    let audit = AuditEvent {
                        event_id: stable_id("audit", &(&atxn.atxn_id, "artifact.publish_local"))?,
                        tenant_id: identity.tenant_id.clone(),
                        actor_id: identity.subject_id.clone(),
                        action: "artifact.publish_local".into(),
                        target_type: "artifact".into(),
                        target_id: artifact_id,
                        request_id: Some(atxn.request_id.clone()),
                        atxn_id: Some(atxn.atxn_id.clone()),
                        outcome: "pass".into(),
                        policy_epoch: identity.policy_epoch,
                        details: json!({"recovered": true}),
                        created_at: atxn.created_at,
                    };
                    atxn = self.store.commit_local_publication(
                        &atxn,
                        &mut result.artifact,
                        &mut result.claims,
                        outcome,
                        &audit,
                    )?;
                }
                AtxnState::NeedsReview => {
                    let result = self.load_result(&identity.tenant_id, &atxn.atxn_id)?;
                    let obligations_satisfied = !result.claims.is_empty()
                        && result.claims.iter().all(|claim| {
                            matches!(
                                claim.review_state,
                                ReviewState::Verified | ReviewState::Approved
                            )
                        });
                    if obligations_satisfied {
                        atxn = self.advance(&atxn, AtxnState::Revalidating, None)?;
                    } else {
                        return Ok(ResumeOutcome::Completed(Box::new(result)));
                    }
                }
                AtxnState::Published => {
                    return Ok(ResumeOutcome::Completed(Box::new(
                        self.load_result(&identity.tenant_id, &atxn.atxn_id)?,
                    )));
                }
                AtxnState::ObjectFailed
                | AtxnState::PublicationFailed
                | AtxnState::RevocationPending => {
                    return Err(AmosError::Conflict(format!(
                        "transaction recovery from {:?} requires a publication retry or revocation adapter acknowledgment",
                        atxn.state
                    )));
                }
                AtxnState::Rejected | AtxnState::Aborted | AtxnState::Revoked => {
                    return Err(AmosError::Conflict(format!(
                        "terminal transaction {:?} cannot be recovered",
                        atxn.state
                    )));
                }
            }
        }
    }

    fn recovery_manifest(&self, atxn: &AnalyticalTransaction) -> Result<ContextManifest> {
        self.store
            .get_manifest_by_atxn(&atxn.tenant_id, &atxn.atxn_id)?
            .ok_or_else(|| AmosError::NotFound("recovery context manifest".into()))
    }

    fn recovery_plan(&self, atxn: &AnalyticalTransaction) -> Result<TypedPlan> {
        self.store
            .get_plan_by_atxn(&atxn.tenant_id, &atxn.atxn_id)?
            .ok_or_else(|| AmosError::NotFound("recovery plan".into()))
    }

    fn prepare_evidence(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        definition: &TaskDefinition,
    ) -> Result<PreparedEvidence> {
        let manifest = self.recovery_manifest(atxn)?;
        let plan = self.recovery_plan(atxn)?;
        let executions = self.recovery_executions(&identity.tenant_id, atxn)?;
        if plan.steps.iter().any(|step| {
            !executions
                .iter()
                .any(|execution| execution.step_id == step.step_id)
        }) {
            return Err(AmosError::Conflict(
                "recovery cannot compose until every plan step has an execution".into(),
            ));
        }
        let (artifact, mut claims, edges) = self.compose(atxn, &manifest, &executions)?;
        let mut verifications = self
            .store
            .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
            .into_iter()
            .filter(|verification| verification.profile_version == 1)
            .collect::<Vec<_>>();
        for claim in claims.iter_mut().filter(|claim| {
            matches!(
                claim.claim_type.as_str(),
                "metric_value" | "metric_comparison" | "concentration"
            )
        }) {
            for execution_id in &claim.support_execution_ids {
                let execution = executions
                    .iter()
                    .find(|execution| &execution.execution_id == execution_id)
                    .ok_or_else(|| AmosError::NotFound(execution_id.clone()))?;
                let step = plan
                    .steps
                    .iter()
                    .find(|step| step.step_id == execution.step_id)
                    .ok_or_else(|| AmosError::NotFound(execution.step_id.clone()))?;
                let step_hash = content_hash(step)?;
                claim.verification_ids.extend(
                    verifications
                        .iter()
                        .filter(|verification| verification.input_hash == step_hash)
                        .map(|verification| verification.verification_id.clone()),
                );
            }
            claim.verification_ids.sort();
            claim.verification_ids.dedup();
        }
        let claim_verification = self.verifier.verify_claims(&ClaimVerificationRequest {
            tenant: &identity.tenant_id,
            atxn_id: &atxn.atxn_id,
            profile: &definition.verifier_profile,
            artifact: &artifact,
            manifest: &manifest,
            claims: &claims,
            edges: &edges,
            executions: &executions,
            verifications: &verifications,
        })?;
        self.store.save_verification(&claim_verification)?;
        verifications.push(claim_verification.clone());
        Ok(PreparedEvidence {
            artifact,
            claims,
            edges,
            executions,
            claim_verification,
        })
    }

    fn recovery_executions(
        &self,
        tenant: &str,
        atxn: &AnalyticalTransaction,
    ) -> Result<Vec<ExecutionRecord>> {
        let plan = self.recovery_plan(atxn)?;
        let persisted = self.store.list_executions(tenant, &atxn.atxn_id)?;
        plan.steps
            .iter()
            .map(|step| {
                persisted
                    .iter()
                    .filter(|execution| {
                        execution.step_id == step.step_id
                            && execution.fencing_token <= atxn.state_seq
                    })
                    .max_by_key(|execution| execution.fencing_token)
                    .cloned()
                    .ok_or_else(|| {
                        AmosError::NotFound(format!("recovery execution for step {}", step.step_id))
                    })
            })
            .collect()
    }

    pub fn replay(
        &self,
        identity: &Identity,
        artifact_id: &str,
        idempotency_key: &str,
    ) -> Result<ReplayResult> {
        let (artifact, _, _) = self.get_artifact_for(identity, artifact_id)?;
        if idempotency_key.trim().is_empty() {
            return Err(AmosError::Validation(
                "replay requires an idempotency key".into(),
            ));
        }
        let package = self
            .store
            .get_replay_package(&identity.tenant_id, artifact_id)?
            .ok_or_else(|| AmosError::NotFound("replay package".into()))?;
        if package.retained_until < Utc::now() {
            return Ok(ReplayResult {
                artifact_id: artifact_id.into(),
                original_atxn_id: artifact.atxn_id,
                replay_atxn_id: String::new(),
                status: Outcome::Reject,
                matching_execution_ids: vec![],
                changed_execution_ids: vec![],
                comparisons: vec![],
                warnings: vec![],
                errors: vec!["replay evidence expired".into()],
            });
        }
        let original_transaction = self
            .store
            .get_transaction(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound(artifact.atxn_id.clone()))?;
        let original_plan = self
            .store
            .get_plan(&identity.tenant_id, &package.plan_id)?
            .ok_or_else(|| AmosError::NotFound("replay plan".into()))?;
        let request_hash = content_hash(&json!({
            "artifact_id": artifact_id,
            "package_id": package.package_id,
            "subject_id": identity.subject_id,
            "requested_replay_level": package.replay_level,
        }))?;
        let now = Utc::now();
        let initial = AnalyticalTransaction {
            tenant_id: identity.tenant_id.clone(),
            atxn_id: new_id("atxn"),
            request_id: new_id("req"),
            idempotency_key: idempotency_key.into(),
            request_hash,
            subject_id: identity.subject_id.clone(),
            request: format!("Replay artifact {artifact_id}"),
            task_type: format!("replay:{}", original_transaction.task_type),
            task_version: original_transaction.task_version,
            risk_class: artifact.risk_class,
            budgets: original_transaction.budgets.clone(),
            policy_epoch: identity.policy_epoch,
            source_versions: package.source_versions.clone(),
            state: AtxnState::Admitted,
            state_seq: 0,
            terminal: false,
            outcome: None,
            warnings: vec![],
            errors: vec![],
            created_at: now,
            updated_at: now,
        };
        let mut replay_atxn = self.store.create_transaction(&initial)?;
        if replay_atxn.atxn_id != initial.atxn_id {
            self.policy
                .authorize_transaction_read(identity, &replay_atxn)?;
            return self
                .store
                .get_replay_result(&identity.tenant_id, &replay_atxn.atxn_id)?
                .ok_or_else(|| {
                    AmosError::Conflict("idempotent replay is still in progress".into())
                });
        }
        replay_atxn = self.advance(&replay_atxn, AtxnState::Observing, None)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::Selecting, None)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::Planning, None)?;
        let mut replay_plan = original_plan.clone();
        replay_plan.plan_id = new_id("plan");
        replay_plan.atxn_id = replay_atxn.atxn_id.clone();
        replay_plan.task_definition = format!("replay:{}", original_plan.task_definition);
        replay_plan.model_identity = "deterministic-replay-controller".into();
        self.store.save_plan(&replay_plan)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::Executing, None)?;
        let fence = replay_atxn.state_seq;
        let original_executions = self
            .store
            .list_executions(&identity.tenant_id, &artifact.atxn_id)?;
        let mut matching = vec![];
        let mut changed = vec![];
        let mut comparisons = vec![];
        let mut equivalent = vec![];
        for step in &replay_plan.steps {
            let original = original_executions
                .iter()
                .find(|execution| execution.step_id == step.step_id)
                .ok_or_else(|| {
                    AmosError::Validation(format!(
                        "replay package has no original execution for step {}",
                        step.step_id
                    ))
                })?;
            let expected = package
                .expected_execution_hashes
                .get(&step.step_id)
                .ok_or_else(|| {
                    AmosError::Validation(format!(
                        "replay package has no expected hash for step {}",
                        step.step_id
                    ))
                })?;
            let capability = self
                .capability_issuer
                .issue(identity, &replay_plan, step, fence)?;
            let replayed =
                self.sql_worker
                    .execute(identity, &replay_plan, step, &capability, fence)?;
            let replayed = self.store.save_execution(&replayed)?;
            let (comparison, explanation) = if expected == &replayed.output_hash {
                matching.push(replayed.execution_id.clone());
                (
                    ReplayComparisonKind::Exact,
                    "output hash exactly matches the retained expectation".into(),
                )
            } else if original.output == replayed.output {
                matching.push(replayed.execution_id.clone());
                equivalent.push(replayed.execution_id.clone());
                (
                    ReplayComparisonKind::Equivalent,
                    "structured output is equivalent although the retained hash differs".into(),
                )
            } else {
                changed.push(replayed.execution_id.clone());
                (
                    ReplayComparisonKind::Different,
                    "structured output and output hash differ from retained evidence".into(),
                )
            };
            comparisons.push(ReplayExecutionComparison {
                step_id: step.step_id.clone(),
                original_execution_id: original.execution_id.clone(),
                replay_execution_id: replayed.execution_id,
                expected_output_hash: expected.clone(),
                actual_output_hash: replayed.output_hash,
                comparison,
                explanation,
            });
        }
        replay_atxn = self.advance(&replay_atxn, AtxnState::Composing, None)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::Verifying, None)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::Revalidating, None)?;
        let status = if changed.is_empty() && equivalent.is_empty() {
            Outcome::Pass
        } else {
            Outcome::Warning
        };
        let result = ReplayResult {
            artifact_id: artifact.artifact_id,
            original_atxn_id: original_transaction.atxn_id,
            replay_atxn_id: replay_atxn.atxn_id.clone(),
            status,
            matching_execution_ids: matching,
            changed_execution_ids: changed.clone(),
            comparisons,
            warnings: [
                (!equivalent.is_empty()).then(|| {
                    "one or more replay outputs were equivalent but not hash-identical".into()
                }),
                (!changed.is_empty())
                    .then(|| "one or more replay outputs differed from retained evidence".into()),
            ]
            .into_iter()
            .flatten()
            .collect(),
            errors: vec![],
        };
        replay_atxn = self.store.commit_replay_result(
            &replay_atxn,
            &result,
            &AuditEvent {
                event_id: new_id("audit"),
                tenant_id: identity.tenant_id.clone(),
                actor_id: identity.subject_id.clone(),
                action: "artifact.replay.compare".into(),
                target_type: "artifact".into(),
                target_id: artifact_id.into(),
                request_id: Some(replay_atxn.request_id.clone()),
                atxn_id: Some(replay_atxn.atxn_id.clone()),
                outcome: if result.status == Outcome::Pass {
                    "pass".into()
                } else {
                    "warning".into()
                },
                policy_epoch: identity.policy_epoch,
                details: json!({
                    "original_atxn_id": result.original_atxn_id,
                    "matching_execution_ids": result.matching_execution_ids,
                    "changed_execution_ids": result.changed_execution_ids,
                }),
                created_at: Utc::now(),
            },
        )?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::ObjectFinalizing, None)?;
        replay_atxn = self.advance(&replay_atxn, AtxnState::PublicationPending, None)?;
        let _ = self.advance(&replay_atxn, AtxnState::Published, Some(result.status))?;
        Ok(result)
    }

    pub async fn replay_async(
        &self,
        identity: &Identity,
        artifact_id: String,
        idempotency_key: String,
    ) -> Result<ReplayResult> {
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        let identity = identity.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.replay(&identity, &artifact_id, &idempotency_key)
        })
        .await
        .map_err(|error| AmosError::Storage(format!("replay worker join failed: {error}")))?
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
            parse_time(WINDOW_START)?,
            parse_time(WINDOW_END)?,
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
        let verification =
            self.verifier
                .verify_step(identity, &definition, &manifest, &proposed)?;
        Ok(SqlPreflight {
            manifest_id: manifest.manifest_id,
            referenced_versions: manifest.source_versions,
            verification,
        })
    }

    pub fn revalidate_artifact(&self, identity: &Identity, artifact_id: &str) -> Result<Value> {
        self.policy.authorize_revalidation(identity)?;
        let (_, mut claims, _) = self.get_artifact_for(identity, artifact_id)?;
        let expected_claims = claims.clone();
        let replay = self
            .store
            .get_replay_package(&identity.tenant_id, artifact_id)?;
        let mut changes = vec![];
        for claim in &mut claims {
            let before_semantic = claim.semantic_validity;
            let before_replay = claim.replay_availability;
            let edges =
                self.store
                    .list_edges_from(&identity.tenant_id, "claim", &claim.claim_id)?;
            let mut stale_memory = false;
            for edge in edges
                .iter()
                .filter(|edge| edge.to.endpoint_type == "memory")
            {
                let memory = self.store.get_memory(&identity.tenant_id, &edge.to.id)?;
                if memory.is_none_or(|memory| {
                    memory.status != crate::domain::MemoryStatus::Active
                        || memory.superseded_by.is_some()
                }) {
                    stale_memory = true;
                    break;
                }
            }
            if stale_memory {
                claim.semantic_validity = SemanticValidity::Stale;
            } else if claim.semantic_validity == SemanticValidity::PendingRevalidation {
                claim.semantic_validity = SemanticValidity::Current;
            }
            if replay
                .as_ref()
                .is_none_or(|package| package.retained_until < Utc::now())
            {
                claim.replay_availability = ReplayAvailability::Expired;
            }
            if before_semantic != claim.semantic_validity
                || before_replay != claim.replay_availability
            {
                changes.push(json!({
                    "claim_id": claim.claim_id,
                    "semantic_validity": {"before":before_semantic,"after":claim.semantic_validity},
                    "replay_availability": {"before":before_replay,"after":claim.replay_availability}
                }));
            }
        }
        let audit = AuditEvent {
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
        };
        claims = self.store.commit_claim_validity_updates(
            &expected_claims,
            &claims,
            &audit,
            "artifact.revalidate",
        )?;
        Ok(json!({"artifact_id":artifact_id,"changes":changes,"claims":claims}))
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
        idempotency_key: String,
    ) -> Result<ReviewResult> {
        let permit = self
            .blocking_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AmosError::Storage("blocking execution lane is closed".into()))?;
        let runtime = self.clone();
        let identity = identity.clone();
        let artifact_id = artifact_id.to_string();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AmosError::Storage(error.to_string()))?
                .block_on(runtime.review_artifact_inner(
                    &identity,
                    &artifact_id,
                    claim_ids,
                    decision,
                    comment,
                    correction,
                    authority,
                    idempotency_key,
                ))
        })
        .await
        .map_err(|error| AmosError::Storage(format!("review worker join failed: {error}")))?
    }

    #[allow(clippy::too_many_arguments)]
    async fn review_artifact_inner(
        &self,
        identity: &Identity,
        artifact_id: &str,
        claim_ids: Vec<String>,
        decision: ReviewDecision,
        comment: String,
        correction: Option<Value>,
        authority: Authority,
        idempotency_key: String,
    ) -> Result<ReviewResult> {
        let review: Review = self.evidence.review(
            identity,
            artifact_id,
            claim_ids,
            decision,
            comment,
            correction,
            authority,
            idempotency_key,
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
            self.finalize_object(&atxn)?;
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

    fn finalize_object(&self, atxn: &AnalyticalTransaction) -> Result<()> {
        if atxn.state != AtxnState::ObjectFinalizing {
            return Err(AmosError::InvalidTransition(
                "object promotion requires object_finalizing state".into(),
            ));
        }
        let artifact = self
            .store
            .get_artifact_by_atxn(&atxn.tenant_id, &atxn.atxn_id)?
            .ok_or_else(|| AmosError::NotFound("artifact for object promotion".into()))?;
        let key = format!("{}/{}.md", artifact.tenant_id, artifact.artifact_id);
        self.object_store
            .stage(&key, &artifact.content, &artifact.content_hash)?;
        self.object_store.promote(&key, &artifact.content_hash)?;
        if self.object_store.read(&key)?.as_deref() != Some(artifact.content.as_str()) {
            return Err(AmosError::Conflict(
                "promoted object could not be read back exactly".into(),
            ));
        }
        Ok(())
    }

    fn artifact_transaction(
        &self,
        identity: &Identity,
        artifact: &Artifact,
    ) -> Result<AnalyticalTransaction> {
        self.store
            .get_transaction(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound(artifact.atxn_id.clone()))
    }

    pub async fn connector_health(&self) -> Result<crate::domain::ConnectorHealth> {
        self.connector.health().await
    }

    pub fn process_one_job(
        &self,
        tenant: &str,
        worker_id: &str,
        lease_seconds: i64,
    ) -> Result<Option<Job>> {
        let Some(job) = self.scheduler.acquire(tenant, worker_id, lease_seconds)? else {
            return Ok(None);
        };
        let fence = job.fencing_token;
        let execution = match job.job_type.as_str() {
            "invalidation.continue" => self.process_invalidation_continuation(&job),
            "claim.revalidate" => self.process_claim_revalidation(&job),
            other => Err(AmosError::Validation(format!(
                "no worker is registered for job type {other}"
            ))),
        };
        let finished = match execution {
            Ok(()) => self.scheduler.complete(job, fence)?,
            Err(error) => self.scheduler.fail(job, fence, error.to_string())?,
        };
        Ok(Some(finished))
    }

    pub fn process_job_batch(
        &self,
        tenant: &str,
        worker_id: &str,
        lease_seconds: i64,
        max_jobs: usize,
        shutdown: &AtomicBool,
    ) -> Result<Vec<Job>> {
        if max_jobs == 0 {
            return Err(AmosError::Validation(
                "job batch must allow at least one job".into(),
            ));
        }
        let mut processed = Vec::new();
        while processed.len() < max_jobs && !shutdown.load(Ordering::Acquire) {
            let Some(job) = self.process_one_job(tenant, worker_id, lease_seconds)? else {
                break;
            };
            processed.push(job);
        }
        Ok(processed)
    }

    fn process_invalidation_continuation(&self, job: &Job) -> Result<()> {
        let target_type = job_payload_string(job, "target_type")?;
        let target_id = job_payload_string(job, "target_id")?;
        let reason = job_payload_string(job, "reason")?;
        let root_key = job_payload_string(job, "invalidation_key")?;
        let after_claim_id = job_payload_string(job, "after_claim_id")?;
        let page_size = job_payload_usize(job, "page_size")?.min(250);
        let traversal_node_quota = job
            .payload
            .get("traversal_node_quota")
            .and_then(Value::as_u64)
            .map_or(Ok(10_000), |value| {
                usize::try_from(value)
                    .map_err(|_| AmosError::Validation("traversal quota is too large".into()))
            })?;
        self.store.invalidate_claims_page_after(
            &job.tenant_id,
            target_type,
            target_id,
            reason,
            &job.idempotency_key,
            root_key,
            Some(after_claim_id),
            page_size,
            traversal_node_quota,
        )?;
        Ok(())
    }

    fn process_claim_revalidation(&self, job: &Job) -> Result<()> {
        let audit_id = format!("audit_job_{}", job.job_id);
        if self.store.has_audit_event(&job.tenant_id, &audit_id)? {
            return Ok(());
        }
        let expected = if let Some(claim_id) = job.payload.get("claim_id").and_then(Value::as_str) {
            vec![
                self.store
                    .get_claim(&job.tenant_id, claim_id)?
                    .ok_or_else(|| AmosError::NotFound(claim_id.into()))?,
            ]
        } else if let Some(artifact_id) = job.payload.get("artifact_id").and_then(Value::as_str) {
            self.store.list_claims(&job.tenant_id, artifact_id)?
        } else {
            return Err(AmosError::Validation(
                "claim revalidation job requires claim_id or artifact_id".into(),
            ));
        };
        if expected.is_empty() {
            return Err(AmosError::NotFound(
                "claim revalidation target has no claims".into(),
            ));
        }
        let mut updated = expected.clone();
        for claim in &mut updated {
            if claim.semantic_validity != SemanticValidity::PendingRevalidation {
                continue;
            }
            let edges = self
                .store
                .list_edges_from(&job.tenant_id, "claim", &claim.claim_id)?;
            let mut stale = false;
            for edge in edges
                .iter()
                .filter(|edge| edge.to.endpoint_type == "memory")
            {
                let memory = self.store.get_memory(&job.tenant_id, &edge.to.id)?;
                if memory.is_none_or(|memory| {
                    memory.status != crate::domain::MemoryStatus::Active
                        || memory.superseded_by.is_some()
                }) {
                    stale = true;
                    break;
                }
            }
            claim.semantic_validity = if stale {
                SemanticValidity::Stale
            } else {
                SemanticValidity::Current
            };
        }
        let artifact_id = expected[0].artifact_id.clone();
        self.store.commit_claim_validity_updates(
            &expected,
            &updated,
            &AuditEvent {
                event_id: audit_id,
                tenant_id: job.tenant_id.clone(),
                actor_id: "system:claim-revalidator".into(),
                action: "claim.revalidate.worker".into(),
                target_type: "artifact".into(),
                target_id: artifact_id,
                request_id: None,
                atxn_id: None,
                outcome: if expected == updated {
                    "pass".into()
                } else {
                    "warning".into()
                },
                policy_epoch: 0,
                details: json!({
                    "job_id": job.job_id,
                    "fencing_token": job.fencing_token,
                    "claim_count": expected.len(),
                }),
                created_at: Utc::now(),
            },
            &job.idempotency_key,
        )?;
        Ok(())
    }

    pub async fn process_source_events(
        &self,
        identity: &Identity,
        cursor: Option<&str>,
    ) -> Result<BTreeMap<String, Vec<String>>> {
        let page = self.connector.subscribe(cursor).await?;
        let memory = self.store.list_active_memory(&identity.tenant_id)?;
        let mut impacted = BTreeMap::new();
        for event in page.items {
            if event.tenant_id != identity.tenant_id {
                return Err(AmosError::PermissionDenied(
                    "connector event crossed the authenticated tenant boundary".into(),
                ));
            }
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
                claims.extend(self.evidence.invalidate_memory_with_key(
                    &event.tenant_id,
                    &object.object_id,
                    &event.change_kind,
                    &format!("source/{}/{}", event.event_id, object.object_id),
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
    fn build_plan(
        &self,
        atxn: &AnalyticalTransaction,
        manifest: &ContextManifest,
    ) -> Result<TypedPlan> {
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
        Ok(TypedPlan {
            plan_id: stable_id(
                "plan",
                &(&atxn.tenant_id, &atxn.atxn_id, "payment_health:v1"),
            )?,
            tenant_id: atxn.tenant_id.clone(),
            atxn_id: atxn.atxn_id.clone(),
            task_definition: manifest.task_definition.clone(),
            manifest_id: manifest.manifest_id.clone(),
            model_identity: "deterministic-alpha-planner:v1".into(),
            steps,
        })
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
            required_u64(current, "failures", "current summary")?,
            required_u64(current, "attempts", "current summary")?,
            required_u64(baseline, "failures", "baseline summary")?,
            required_u64(baseline, "attempts", "baseline summary")?,
        )?;
        let current_rate = required_f64(&comparison, "current_rate", "rate comparison")?;
        let baseline_rate = required_f64(&comparison, "baseline_rate", "rate comparison")?;
        let concentration = find_execution(executions, "concentration")?;
        let top = concentration
            .output
            .as_array()
            .and_then(|r| r.first())
            .ok_or_else(|| AmosError::Execution("concentration output empty".into()))?;
        let timeseries = find_execution(executions, "timeseries")?;
        let points: Vec<(String, f64)> = timeseries
            .output
            .as_array()
            .ok_or_else(|| AmosError::Execution("timeseries output is not rows".into()))?
            .iter()
            .enumerate()
            .map(|(index, row)| {
                Ok((
                    required_str(row, "hour", &format!("timeseries row {index}"))?.to_string(),
                    required_f64(row, "failure_rate", &format!("timeseries row {index}"))?,
                ))
            })
            .collect::<Result<_>>()?;
        let processor = required_str(top, "processor", "concentration row")?;
        let card_network = required_str(top, "card_network", "concentration row")?;
        let concentration_rate = required_f64(top, "failure_rate", "concentration row")?;
        let (svg, chart_hash) = self.charts.timeseries_svg(&points)?;
        let artifact_id = stable_id("art", &(&atxn.tenant_id, &atxn.atxn_id, "report:v3"))?;
        let report = format!(
            "# Payment failure health review\n\nFailure rate increased from {:.1}% to {:.1}%. The largest concentration was {} / {} at {:.1}%. A gateway deployment preceded the spike, but causality and the dashboard action require review.\n\nChart hash: `{}`\n\n{}",
            baseline_rate * 100.0,
            current_rate * 100.0,
            processor,
            card_network,
            concentration_rate * 100.0,
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
            content_hash: content_hash(&report)?,
            audience: "internal".into(),
            risk_class: RiskClass::MaterialInternal,
            object_state: "finalized".into(),
            publication_validity: PublicationValidity::Draft,
            created_at: atxn.created_at,
        };
        let claims = vec![
            claim(
                &artifact,
                "metric_comparison",
                format!(
                    "Payment failure rate increased from {:.1}% to {:.1}%.",
                    baseline_rate * 100.0,
                    current_rate * 100.0
                ),
                json!({"metric_id":"payment_failure_rate:v3","current_value":current_rate,"baseline_value":baseline_rate}),
                vec![summary.execution_id.clone()],
            )?,
            claim(
                &artifact,
                "concentration",
                format!("The largest failure concentration was {processor} / {card_network}."),
                top.clone(),
                vec![concentration.execution_id.clone()],
            )?,
            claim(
                &artifact,
                "causal",
                "The gateway deployment may have contributed to the spike.".into(),
                json!({"review_required":true}),
                vec![],
            )?,
            claim(
                &artifact,
                "operational_recommendation",
                "Update the executive dashboard with a warning while the cause remains under review."
                    .into(),
                json!({"review_required":true}),
                vec![],
            )?,
        ];
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
                )?);
            }
            edges.push(edge(
                atxn,
                &item.claim_id,
                "governed_by_metric",
                "memory",
                &metric,
                None,
            )?);
            edges.push(edge(
                atxn,
                &item.claim_id,
                "governed_by_schema",
                "memory",
                &schema,
                None,
            )?);
            edges.push(edge(
                atxn,
                &item.claim_id,
                "scoped_to_data_state",
                "memory",
                &data,
                None,
            )?);
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
                )?);
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
    ) -> Result<ReplayPackage> {
        Ok(ReplayPackage {
            package_id: stable_id(
                "rpl",
                &(&artifact.tenant_id, &artifact.artifact_id, "level3"),
            )?,
            tenant_id: artifact.tenant_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            replay_level: 3,
            manifest_id: manifest.manifest_id.clone(),
            plan_id: plan.plan_id.clone(),
            execution_ids: executions.iter().map(|e| e.execution_id.clone()).collect(),
            template: "payment_health_report:v3".into(),
            render_config_hash: content_hash(&"payment_health_report:v3")?,
            retained_until: artifact.created_at + Duration::days(365),
            expected_artifact_hash: artifact.content_hash.clone(),
            expected_execution_hashes: executions
                .iter()
                .map(|e| (e.step_id.clone(), e.output_hash.clone()))
                .collect(),
            source_versions: manifest.source_versions.clone(),
        })
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
) -> Result<Claim> {
    Ok(Claim {
        tenant_id: artifact.tenant_id.clone(),
        claim_id: stable_id(
            "clm",
            &(&artifact.tenant_id, &artifact.artifact_id, claim_type),
        )?,
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
    })
}
fn edge(
    atxn: &AnalyticalTransaction,
    claim: &str,
    relation: &str,
    target_type: &str,
    target: &str,
    version: Option<String>,
) -> Result<DependencyEdge> {
    let mut e = DependencyEdge {
        edge_id: stable_id(
            "edge",
            &(
                &atxn.tenant_id,
                &atxn.atxn_id,
                claim,
                relation,
                target_type,
                target,
            ),
        )?,
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
    e.content_hash = content_hash(&e)?;
    Ok(e)
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

fn required_u64(value: &Value, field: &str, context: &str) -> Result<u64> {
    value.get(field).and_then(Value::as_u64).ok_or_else(|| {
        AmosError::Execution(format!("{context} requires unsigned integer field {field}"))
    })
}

fn required_f64(value: &Value, field: &str, context: &str) -> Result<f64> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| {
            AmosError::Execution(format!("{context} requires finite numeric field {field}"))
        })
}

fn required_str<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            AmosError::Execution(format!("{context} requires non-empty string field {field}"))
        })
}

fn job_payload_string<'a>(job: &'a Job, field: &str) -> Result<&'a str> {
    job.payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AmosError::Validation(format!(
                "job {} requires non-empty string field {field}",
                job.job_id
            ))
        })
}

fn job_payload_usize(job: &Job, field: &str) -> Result<usize> {
    let value = job
        .payload
        .get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AmosError::Validation(format!(
                "job {} requires positive integer field {field}",
                job.job_id
            ))
        })?;
    usize::try_from(value)
        .map_err(|_| AmosError::Validation(format!("job field {field} is too large")))
}

fn parse_time(value: &str) -> Result<chrono::DateTime<Utc>> {
    Ok(chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|error| AmosError::Validation(format!("invalid scenario timestamp: {error}")))?
        .with_timezone(&Utc))
}
