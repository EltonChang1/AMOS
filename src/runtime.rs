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
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::{
    Result,
    artifacts::{
        VerifiedFactCatalog, build_fact_catalog, compile_artifact, validate_narrative_plan,
    },
    connectors::{Connector, Page, SqliteWarehouseConnector},
    context::ContextCompiler,
    domain::{
        AnalyticalTransaction, Artifact, AtxnState, AuditEvent, Authority, Claim, ContextManifest,
        DependencyEdge, ErasureReceipt, ExecutionRecord, Identity, Job, MemoryObject, MemoryType,
        OutboxEvent, Outcome, PlanStep, ReplayAvailability, ReplayComparisonKind,
        ReplayExecutionComparison, ReplayPackage, ReplayResult, RetentionCommand, RetentionRecord,
        Review, ReviewDecision, ReviewResult, ReviewState, RunResult, SemanticValidity,
        SqlPreflight, TaskDefinition, TypedPlan, VerificationRecord, content_hash, new_id,
        stable_id,
    },
    error::AmosError,
    evidence::EvidenceService,
    memory::MemoryService,
    model::{
        AnalysisKind, ModelDescriptor, ModelGenerationConfig, ModelInvocationStatus, ModelInvoker,
        ModelProvider, ModelPurpose, ModelRequestTemplate, NarrativePlan, PlanProposal,
        UnavailableModelProvider, narrative_response_schema_for_evidence,
        plan_response_schema_for_relation,
    },
    observability::{MetricsSnapshot, OperationalMetrics},
    packs::AnalysisPack,
    policy::PolicyEngine,
    privacy::PrivacyBoundaryConfig,
    publication::{LocalFilesystemObjectStore, ObjectStore},
    scheduler::Scheduler,
    seed::TENANT,
    store::Store,
    verification::{ClaimVerificationRequest, Verifier},
    workers::{CapabilityIssuer, SqlWorker},
};

#[derive(Clone)]
pub struct RuntimeConfig {
    pub control_db: PathBuf,
    pub warehouse_db: PathBuf,
    pub object_root: PathBuf,
    pub privacy: PrivacyBoundaryConfig,
    pub analysis_pack: AnalysisPack,
    pub model_max_attempts: u32,
    pub model_temperature: f32,
    capability_key: Vec<u8>,
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("control_db", &self.control_db)
            .field("warehouse_db", &self.warehouse_db)
            .field("object_root", &self.object_root)
            .field("privacy", &self.privacy)
            .field("analysis_pack", &self.analysis_pack.pack_id)
            .field("model_max_attempts", &self.model_max_attempts)
            .field("model_temperature", &self.model_temperature)
            .field("capability_key", &"[REDACTED]")
            .finish()
    }
}

impl RuntimeConfig {
    pub fn new(
        control_db: impl Into<PathBuf>,
        warehouse_db: impl Into<PathBuf>,
        capability_key: impl Into<Vec<u8>>,
        analysis_pack: AnalysisPack,
    ) -> Result<Self> {
        analysis_pack.validate()?;
        let control_db = control_db.into();
        let warehouse_db = warehouse_db.into();
        let object_root = control_db
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("objects");
        Ok(Self {
            control_db,
            warehouse_db,
            object_root,
            privacy: PrivacyBoundaryConfig::local_air_gapped(),
            analysis_pack,
            model_max_attempts: 2,
            model_temperature: 0.1,
            capability_key: capability_key.into(),
        })
    }

    pub fn demo(root: impl AsRef<Path>) -> Result<Self> {
        Self::new(
            root.as_ref().join("data/amos.sqlite"),
            root.as_ref().join("data/warehouse.sqlite"),
            b"amos-explicit-demo-capability-key-v1".to_vec(),
            AnalysisPack::subscription_churn()?,
        )
    }

    pub fn with_analysis_pack(mut self, analysis_pack: AnalysisPack) -> Result<Self> {
        analysis_pack.validate()?;
        self.analysis_pack = analysis_pack;
        Ok(self)
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
    blocking_permits: Arc<Semaphore>,
    metrics: Arc<OperationalMetrics>,
    object_store: LocalFilesystemObjectStore,
    analysis_pack: Arc<AnalysisPack>,
    privacy: PrivacyBoundaryConfig,
    model_generation: ModelGenerationConfig,
    model: ModelInvoker,
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

#[derive(Debug, Clone, Serialize)]
pub struct DemoSourceChangeResult {
    pub superseded_memory_id: String,
    pub successor_memory_id: String,
    pub previous_source_version: String,
    pub current_source_version: String,
    pub affected_artifact_ids: Vec<String>,
    pub affected_claim_ids: Vec<String>,
    pub jobs: Vec<Job>,
    pub outbox: Vec<OutboxEvent>,
    pub audit: Vec<AuditEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimEvidenceView {
    pub claim: Claim,
    pub dependencies: Vec<DependencyEdge>,
    pub transaction: AnalyticalTransaction,
    pub artifact: Artifact,
    pub manifest: ContextManifest,
    pub plan: TypedPlan,
    pub executions: Vec<ExecutionRecord>,
    pub verifications: Vec<VerificationRecord>,
    pub governed_objects: Vec<MemoryObject>,
    pub model_invocations: Vec<ModelInvocationEvidence>,
    pub audit: Vec<AuditEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInvocationEvidence {
    pub invocation_id: String,
    pub purpose: ModelPurpose,
    pub provider: String,
    pub model: String,
    pub route_class: crate::privacy::ModelRouteClass,
    pub input_manifest_hash: String,
    pub input_payload_hash: String,
    pub output_hash: Option<String>,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub status: ModelInvocationStatus,
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
        Self::open_with_model(
            config,
            Arc::new(UnavailableModelProvider::new(
                "no model provider was configured",
            )),
        )
    }

    pub fn open_with_model(
        config: RuntimeConfig,
        model_provider: Arc<dyn ModelProvider>,
    ) -> Result<Self> {
        config.privacy.validate()?;
        if !config.model_temperature.is_finite() || !(0.0..=2.0).contains(&config.model_temperature)
        {
            return Err(AmosError::Validation(
                "model temperature must be finite and between zero and two".into(),
            ));
        }
        let store = Store::open(&config.control_db)?;
        let privacy = config.privacy.clone();
        let policy = PolicyEngine;
        let memory = MemoryService::new(store.clone(), policy.clone());
        let scheduler = Scheduler::new(store.clone());
        let evidence = EvidenceService::new(store.clone(), policy.clone());
        let context = ContextCompiler::new(memory.clone());
        let issuer = CapabilityIssuer::new(config.capability_key)?;
        let (primary_relation, primary_source) = {
            let (relation, source) = config.analysis_pack.primary_relation()?;
            (relation.to_string(), source.to_string())
        };
        let connector_permissions = config
            .analysis_pack
            .schemas
            .iter()
            .find(|schema| schema.relation == primary_relation)
            .map(|schema| schema.permission_labels.clone())
            .ok_or_else(|| {
                AmosError::Validation("analysis pack primary schema is missing".into())
            })?;
        let connector = Arc::new(SqliteWarehouseConnector::new(
            TENANT,
            primary_source,
            &config.warehouse_db,
            issuer.clone(),
            connector_permissions,
        ));
        let sql_worker = SqlWorker::new(&config.warehouse_db, issuer.clone());
        let object_store = LocalFilesystemObjectStore::new(&config.object_root)?;
        let model = ModelInvoker::new(model_provider, store.clone(), config.model_max_attempts)?;
        let analysis_pack = Arc::new(config.analysis_pack);
        let model_generation = ModelGenerationConfig {
            temperature: config.model_temperature,
            max_output_tokens: 2_048,
        };
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
            blocking_permits: Arc::new(Semaphore::new(8)),
            metrics: Arc::new(OperationalMetrics::default()),
            object_store,
            analysis_pack,
            privacy,
            model_generation,
            model,
        })
    }

    pub fn metrics(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn privacy_boundary(&self) -> Result<crate::privacy::PrivacyBoundaryView> {
        self.privacy.view()
    }

    pub fn model_descriptor(&self) -> ModelDescriptor {
        self.model.descriptor()
    }

    pub fn model_compatibility_probe_passed(&self) -> Result<bool> {
        self.store.model_compatibility_probe_passed(TENANT)
    }

    pub fn default_analysis_pack(&self) -> &AnalysisPack {
        &self.analysis_pack
    }

    pub fn pack_for_task(
        &self,
        tenant: &str,
        task_type: &str,
        version: Option<u32>,
    ) -> Result<Arc<AnalysisPack>> {
        let installed = match version {
            Some(version) => self
                .store
                .get_analysis_pack_by_task_type_version(tenant, task_type, version)?,
            None => self
                .store
                .get_analysis_pack_by_task_type(tenant, task_type)?,
        };
        if let Some(pack) = installed {
            return Ok(Arc::new(pack));
        }
        if self.analysis_pack.task_type == task_type
            && version.is_none_or(|value| value == self.analysis_pack.version)
        {
            return Ok(self.analysis_pack.clone());
        }
        Err(AmosError::NotFound(format!(
            "analysis pack for task type {task_type}"
        )))
    }

    pub fn install_pack(
        &self,
        identity: &Identity,
        pack: AnalysisPack,
    ) -> Result<(bool, AnalysisPack)> {
        self.policy.authorize_operations(identity)?;
        pack.validate()?;
        let installed = self.store.install_analysis_pack(
            &identity.tenant_id,
            &pack,
            &identity.subject_id,
            Utc::now(),
        )?;
        self.store.append_audit(&AuditEvent {
            event_id: new_id("audit"),
            tenant_id: identity.tenant_id.clone(),
            actor_id: identity.subject_id.clone(),
            action: "pack.install".into(),
            target_type: "analysis_pack".into(),
            target_id: format!("{}:v{}", pack.pack_id, pack.version),
            request_id: None,
            atxn_id: None,
            outcome: if installed { "created" } else { "idempotent" }.into(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "pack_id": pack.pack_id,
                "task_type": pack.task_type,
                "version": pack.version,
                "newly_installed": installed,
            }),
            created_at: Utc::now(),
        })?;
        Ok((installed, pack))
    }

    pub fn list_installed_packs(
        &self,
        identity: &Identity,
    ) -> Result<Vec<crate::store::AnalysisPackRecord>> {
        self.store.list_analysis_packs(&identity.tenant_id)
    }

    pub fn get_installed_pack(&self, identity: &Identity, pack_id: &str) -> Result<AnalysisPack> {
        self.store
            .get_analysis_pack(&identity.tenant_id, pack_id)?
            .ok_or_else(|| AmosError::NotFound(format!("analysis pack {pack_id}")))
    }

    pub fn blocked_analysis_fields(&self) -> Vec<String> {
        self.analysis_pack
            .schemas
            .iter()
            .flat_map(|schema| schema.blocked_columns.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
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

    pub fn claim_evidence_view(
        &self,
        identity: &Identity,
        claim_id: &str,
    ) -> Result<ClaimEvidenceView> {
        let (claim, dependencies) = self.get_claim_for(identity, claim_id)?;
        let artifact = self
            .store
            .get_artifact(&identity.tenant_id, &claim.artifact_id)?
            .ok_or_else(|| AmosError::NotFound(claim.artifact_id.clone()))?;
        let transaction = self.get_transaction_for(identity, &artifact.atxn_id)?;
        let manifest = self
            .store
            .get_manifest_by_atxn(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound("claim context manifest".into()))?;
        let plan = self
            .store
            .get_plan_by_atxn(&identity.tenant_id, &artifact.atxn_id)?
            .ok_or_else(|| AmosError::NotFound("claim typed plan".into()))?;
        let executions = self
            .store
            .list_executions(&identity.tenant_id, &artifact.atxn_id)?
            .into_iter()
            .filter(|execution| {
                claim
                    .support_execution_ids
                    .contains(&execution.execution_id)
            })
            .collect::<Vec<_>>();
        let verifications = self
            .store
            .list_verifications(&identity.tenant_id, &artifact.atxn_id)?
            .into_iter()
            .filter(|verification| {
                claim
                    .verification_ids
                    .contains(&verification.verification_id)
            })
            .collect::<Vec<_>>();
        let governed_ids = dependencies
            .iter()
            .filter(|edge| edge.to.endpoint_type == "memory")
            .map(|edge| edge.to.id.as_str())
            .collect::<BTreeSet<_>>();
        let governed_objects = manifest
            .selected_objects
            .iter()
            .filter(|object| governed_ids.contains(object.object_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let model_invocations = self
            .store
            .list_model_invocations(&identity.tenant_id, &artifact.atxn_id)?
            .into_iter()
            .map(|invocation| ModelInvocationEvidence {
                invocation_id: invocation.invocation_id,
                purpose: invocation.purpose,
                provider: invocation.provider,
                model: invocation.model,
                route_class: invocation.route_class,
                input_manifest_hash: invocation.input_manifest_hash,
                input_payload_hash: invocation.input_payload_hash,
                output_hash: invocation.output_hash,
                latency_ms: invocation.latency_ms,
                input_tokens: invocation.input_tokens,
                output_tokens: invocation.output_tokens,
                status: invocation.status,
            })
            .collect();
        let audit = self
            .store
            .list_audit(&identity.tenant_id, 500)?
            .into_iter()
            .filter(|event| {
                event.target_id == claim.claim_id
                    || event.target_id == artifact.artifact_id
                    || event.atxn_id.as_deref() == Some(artifact.atxn_id.as_str())
                    || event
                        .details
                        .get("affected_claim_ids")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| {
                            ids.iter()
                                .any(|id| id.as_str() == Some(claim.claim_id.as_str()))
                        })
            })
            .collect();
        Ok(ClaimEvidenceView {
            claim,
            dependencies,
            transaction,
            artifact,
            manifest,
            plan,
            executions,
            verifications,
            governed_objects,
            model_invocations,
            audit,
        })
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
        self.run_task_typed(identity, request, None, idempotency_key)
            .await
    }

    pub async fn run_task_typed(
        &self,
        identity: &Identity,
        request: String,
        task_type: Option<String>,
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
                .block_on(runtime.run_task_inner(&identity, request, task_type, idempotency_key))
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
        task_type: Option<String>,
        idempotency_key: String,
    ) -> Result<RunResult> {
        let task_type = task_type.unwrap_or_else(|| self.analysis_pack.task_type.clone());
        let pack = self.pack_for_task(&identity.tenant_id, &task_type, None)?;
        let definition = self
            .store
            .get_task_definition(&identity.tenant_id, &pack.task_type)?
            .ok_or_else(|| AmosError::NotFound(format!("{} task definition", pack.task_type)))?;
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
        let atxn = self.store.create_transaction(&initial)?;
        if atxn.atxn_id != initial.atxn_id {
            self.policy.authorize_transaction_read(identity, &atxn)?;
        }
        match atxn.state {
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
        }
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
        let pack = self.pack_for_task(
            &identity.tenant_id,
            &atxn.task_type,
            Some(atxn.task_version),
        )?;
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
                    let (relation, source) = pack.primary_relation()?;
                    let relation = relation.to_string();
                    let source_key = format!("{source}:{relation}");
                    if !atxn.source_versions.contains_key(&source_key) {
                        let observation =
                            self.connector.observe(&format!("table:{relation}")).await?;
                        atxn.source_versions.insert(
                            format!("{}:{relation}", observation.source_id),
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
                                pack.start()?,
                                pack.end()?,
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
                        let mut plan = match self
                            .propose_plan(identity, &atxn, &definition, &manifest, &pack)
                            .await
                        {
                            Ok(plan) => plan,
                            Err(error) => {
                                self.append_model_failure_audit(
                                    identity,
                                    &atxn,
                                    ModelPurpose::Plan,
                                )?;
                                let _ = self.advance(
                                    &atxn,
                                    AtxnState::Rejected,
                                    Some(Outcome::Reject),
                                )?;
                                return Err(error);
                            }
                        };
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
                                self.append_verification_audit(identity, &atxn, &verification)?;
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
                    let admitted_plan = self.recovery_plan(&atxn)?;
                    self.append_plan_admission_audit(identity, &atxn, &admitted_plan)?;
                    for verification in self
                        .store
                        .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
                        .iter()
                        .filter(|verification| verification.profile_version == 1)
                    {
                        self.append_verification_audit(identity, &atxn, verification)?;
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
                        let mut execution = self.sql_worker.execute(
                            identity,
                            &plan,
                            step,
                            &capability,
                            atxn.state_seq,
                        )?;
                        execution.input_versions = atxn.source_versions.clone();
                        self.store.save_execution(&execution)?;
                    }
                    for execution in self.recovery_executions(&identity.tenant_id, &atxn)? {
                        self.append_execution_audit(identity, &atxn, &execution)?;
                    }
                    atxn = self.advance(&atxn, AtxnState::Composing, None)?;
                }
                AtxnState::Composing => {
                    let manifest = self.recovery_manifest(&atxn)?;
                    let executions = self.recovery_executions(&identity.tenant_id, &atxn)?;
                    let plan = self.recovery_plan(&atxn)?;
                    let verifications = self
                        .store
                        .list_verifications(&identity.tenant_id, &atxn.atxn_id)?;
                    let catalog = build_fact_catalog(
                        &atxn,
                        &manifest,
                        &plan,
                        &executions,
                        &verifications,
                        &pack,
                    )?;
                    self.store.save_verified_fact_catalog(&catalog)?;
                    atxn = self.advance(&atxn, AtxnState::Verifying, None)?;
                }
                AtxnState::Verifying => {
                    let candidate = match self
                        .prepare_evidence(identity, &atxn, &definition, &pack)
                        .await
                    {
                        Ok(candidate) => candidate,
                        Err(error)
                            if matches!(
                                &error,
                                AmosError::ModelUnavailable(_)
                                    | AmosError::ModelTimeout
                                    | AmosError::ModelOutputInvalid(_)
                            ) =>
                        {
                            self.append_model_failure_audit(
                                identity,
                                &atxn,
                                ModelPurpose::Narrative,
                            )?;
                            let _ =
                                self.advance(&atxn, AtxnState::Rejected, Some(Outcome::Reject))?;
                            return Err(error);
                        }
                        Err(error) => return Err(error),
                    };
                    for verification in self
                        .store
                        .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
                    {
                        self.append_verification_audit(identity, &atxn, &verification)?;
                    }
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
                    let (relation, source) = pack.primary_relation()?;
                    let source_key = format!("{source}:{relation}");
                    let relation_subject = format!("table:{relation}");
                    if self
                        .store
                        .get_artifact_by_atxn(&identity.tenant_id, &atxn.atxn_id)?
                        .is_some()
                    {
                        let observed = atxn.source_versions.get(&source_key).ok_or_else(|| {
                            AmosError::Validation(
                                "review recovery has no source observation".into(),
                            )
                        })?;
                        let validation =
                            self.connector.validate(&relation_subject, observed).await?;
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
                        None => {
                            self.prepare_evidence(identity, &atxn, &definition, &pack)
                                .await?
                        }
                    };
                    let observed = atxn.source_versions.get(&source_key).ok_or_else(|| {
                        AmosError::Validation(
                            "recovery checkpoint has no source observation".into(),
                        )
                    })?;
                    let validation = self.connector.validate(&relation_subject, observed).await?;
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
                        &pack,
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

    async fn prepare_evidence(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        definition: &TaskDefinition,
        pack: &AnalysisPack,
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
        let mut verifications = self
            .store
            .list_verifications(&identity.tenant_id, &atxn.atxn_id)?
            .into_iter()
            .filter(|verification| verification.profile_version == 1)
            .collect::<Vec<_>>();
        let catalog = match self
            .store
            .get_verified_fact_catalog(&identity.tenant_id, &atxn.atxn_id)?
        {
            Some(catalog) => catalog,
            None => self.store.save_verified_fact_catalog(&build_fact_catalog(
                atxn,
                &manifest,
                &plan,
                &executions,
                &verifications,
                pack,
            )?)?,
        };
        let narrative = self
            .generate_narrative(identity, atxn, &manifest, &catalog, pack)
            .await?;
        let (artifact, mut claims, edges) = compile_artifact(
            atxn,
            &manifest,
            &catalog,
            &narrative,
            pack,
            &plan.model_identity,
        )?;
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
        let claim_request = ClaimVerificationRequest {
            tenant: &identity.tenant_id,
            atxn_id: &atxn.atxn_id,
            profile: &definition.verifier_profile,
            artifact: &artifact,
            manifest: &manifest,
            claims: &claims,
            edges: &edges,
            executions: &executions,
            verifications: &verifications,
        };
        let claim_verification = self
            .verifier
            .verify_claims_with_profile(&claim_request, &pack.verifier_profile)?;
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
        self.preflight_sql_for_task(identity, request, sql, None)
    }

    pub fn preflight_sql_for_task(
        &self,
        identity: &Identity,
        request: &str,
        sql: String,
        task_type: Option<&str>,
    ) -> Result<SqlPreflight> {
        let task_type = task_type.unwrap_or(self.analysis_pack.task_type.as_str());
        let pack = self.pack_for_task(&identity.tenant_id, task_type, None)?;
        let definition = self
            .store
            .get_task_definition(&identity.tenant_id, &pack.task_type)?
            .ok_or_else(|| AmosError::NotFound(format!("{} task definition", pack.task_type)))?;
        self.policy.authorize_task(identity, &definition)?;
        let atxn_id = new_id("preflight");
        let manifest = self.context.compile(
            identity,
            &atxn_id,
            request,
            &definition,
            pack.start()?,
            pack.end()?,
        )?;
        if !manifest.conflicts.is_empty() {
            return Err(AmosError::Conflict(
                "context has equal-authority conflicts".into(),
            ));
        }
        let (relation, source_id) = pack.primary_relation()?;
        let proposed = PlanStep {
            step_id: "preflight".into(),
            purpose: "preflight proposed SQL".into(),
            tool: "sql.readonly.v1".into(),
            source_id: source_id.into(),
            input_object_ids: manifest
                .selected_objects
                .iter()
                .map(|object| object.object_id.clone())
                .collect(),
            parameter_schema: "amos.sql-query.v1".into(),
            parameters: json!({"sql":sql,"relations":[relation]}),
            expected_output_schema: "preflight".into(),
            limits: crate::domain::OperationLimits {
                seconds: definition.budgets.max_seconds,
                rows: definition.budgets.max_rows,
                bytes: definition.budgets.max_bytes,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: definition.verifier_profile.clone(),
        };
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
            let pack = self.pack_for_task(
                &identity.tenant_id,
                &atxn.task_type,
                Some(atxn.task_version),
            )?;
            let (relation, source) = pack.primary_relation()?;
            let observed = atxn
                .source_versions
                .get(&format!("{source}:{relation}"))
                .ok_or_else(|| AmosError::Validation("missing source observation".into()))?;
            let validation = self
                .connector
                .validate(&format!("table:{relation}"), observed)
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

    pub fn trigger_demo_source_change(
        &self,
        identity: &Identity,
        idempotency_key: &str,
    ) -> Result<DemoSourceChangeResult> {
        self.policy.authorize_operations(identity)?;
        if idempotency_key.trim().is_empty() || idempotency_key.len() > 160 {
            return Err(AmosError::Validation(
                "the governed source change requires a bounded idempotency key".into(),
            ));
        }
        let (primary_relation, primary_source) = {
            let (relation, source) = self.analysis_pack.primary_relation()?;
            (relation.to_string(), source.to_string())
        };
        let current_source_version = stable_id(
            "snapshot_successor",
            &(identity.tenant_id.as_str(), idempotency_key),
        )?;
        let active_memory = self.store.list_active_memory(&identity.tenant_id)?;
        let active_snapshot = active_memory
            .into_iter()
            .find(|object| {
                object.source_id == primary_source
                    && object
                        .content
                        .get("role")
                        .and_then(Value::as_str)
                        .is_some_and(|role| role == "data_snapshot")
                    && object
                        .content
                        .get("relation")
                        .and_then(Value::as_str)
                        .is_some_and(|relation| relation == primary_relation)
            })
            .ok_or_else(|| AmosError::NotFound("active governed data snapshot".into()))?;
        let (superseded_memory_id, successor) = if active_snapshot.source_version
            == current_source_version
        {
            let superseded_memory_id =
                active_snapshot.supersedes.last().cloned().ok_or_else(|| {
                    AmosError::Conflict(
                        "idempotent demo source successor lacks its predecessor".into(),
                    )
                })?;
            (superseded_memory_id, active_snapshot)
        } else {
            let superseded_memory_id = active_snapshot.object_id.clone();
            let mut successor = active_snapshot;
            successor.object_id = new_id("mem");
            successor.source_version = current_source_version.clone();
            successor.summary = format!(
                "Successor {primary_relation} snapshot received; dependent historical claims require revalidation."
            );
            successor.recorded_at = Utc::now();
            successor.supersedes = vec![superseded_memory_id.clone()];
            successor.superseded_by = None;
            successor.status = crate::domain::MemoryStatus::Active;
            successor.provenance_ref =
                Some(format!("demo-source-successor/{current_source_version}"));
            let content = successor.content.as_object_mut().ok_or_else(|| {
                AmosError::Validation("governed snapshot content must be an object".into())
            })?;
            content.insert(
                "snapshot_id".into(),
                json!(format!("{primary_relation}_{current_source_version}")),
            );
            content.insert(
                "watermark".into(),
                json!(self.analysis_pack.time_window.end),
            );
            content.insert(
                "freshness_warning".into(),
                json!("a governed successor snapshot was received after publication"),
            );
            successor.content_hash = content_hash(&successor.content)?;
            let successor = self
                .memory
                .supersede(identity, &superseded_memory_id, successor)?;
            (superseded_memory_id, successor)
        };
        let invalidation_key = format!("demo-source-change/{idempotency_key}");
        let affected_claim_ids = self.evidence.invalidate_memory_with_key(
            &identity.tenant_id,
            &superseded_memory_id,
            "source_successor_received",
            &invalidation_key,
        )?;
        let shutdown = AtomicBool::new(false);
        self.process_job_batch(
            &identity.tenant_id,
            "demo-source-change-worker",
            30,
            250,
            &shutdown,
        )?;
        let affected_claim_set = affected_claim_ids.iter().cloned().collect::<BTreeSet<_>>();
        let affected_artifact_ids = affected_claim_ids
            .iter()
            .map(|claim_id| {
                self.store
                    .get_claim(&identity.tenant_id, claim_id)?
                    .map(|claim| claim.artifact_id)
                    .ok_or_else(|| AmosError::NotFound(claim_id.clone()))
            })
            .collect::<Result<BTreeSet<_>>>()?
            .into_iter()
            .collect::<Vec<_>>();
        let jobs = self
            .store
            .list_jobs(&identity.tenant_id, 250)?
            .into_iter()
            .filter(|job| {
                job.payload.get("invalidation_key").and_then(Value::as_str)
                    == Some(invalidation_key.as_str())
            })
            .collect::<Vec<_>>();
        let job_ids = jobs
            .iter()
            .map(|job| job.job_id.clone())
            .collect::<BTreeSet<_>>();
        let outbox = self
            .store
            .list_outbox(&identity.tenant_id, 500)?
            .into_iter()
            .filter(|event| {
                event.idempotency_key.contains(&invalidation_key)
                    || affected_claim_set.contains(&event.aggregate_id)
                        && event.event_type == "claim.validity_changed"
            })
            .collect::<Vec<_>>();
        let source_audit_id = stable_id(
            "audit_source_change",
            &(identity.tenant_id.as_str(), idempotency_key),
        )?;
        if !self
            .store
            .has_audit_event(&identity.tenant_id, &source_audit_id)?
        {
            self.store.append_audit(&AuditEvent {
                event_id: source_audit_id,
                tenant_id: identity.tenant_id.clone(),
                actor_id: identity.subject_id.clone(),
                action: "source.successor.receive".into(),
                target_type: "memory".into(),
                target_id: successor.object_id.clone(),
                request_id: None,
                atxn_id: None,
                outcome: "warning".into(),
                policy_epoch: identity.policy_epoch,
                details: json!({
                    "superseded_memory_id":superseded_memory_id,
                    "successor_memory_id":successor.object_id,
                    "previous_source_version":self.store
                        .get_memory(&identity.tenant_id, &superseded_memory_id)?
                        .map(|object|object.source_version),
                    "current_source_version":current_source_version,
                    "affected_artifact_ids":affected_artifact_ids,
                    "affected_claim_ids":affected_claim_ids,
                    "invalidation_key":invalidation_key,
                }),
                created_at: Utc::now(),
            })?;
        }
        let audit = self
            .store
            .list_audit(&identity.tenant_id, 250)?
            .into_iter()
            .filter(|event| {
                event
                    .details
                    .get("invalidation_key")
                    .and_then(Value::as_str)
                    == Some(invalidation_key.as_str())
                    || event
                        .details
                        .get("root_invalidation_key")
                        .and_then(Value::as_str)
                        == Some(invalidation_key.as_str())
                    || event
                        .details
                        .get("job_id")
                        .and_then(Value::as_str)
                        .is_some_and(|job_id| job_ids.contains(job_id))
            })
            .collect::<Vec<_>>();
        let previous_source_version = self
            .store
            .get_memory(&identity.tenant_id, &superseded_memory_id)?
            .map(|object| object.source_version)
            .ok_or_else(|| AmosError::NotFound(superseded_memory_id.clone()))?;
        Ok(DemoSourceChangeResult {
            superseded_memory_id,
            successor_memory_id: successor.object_id,
            previous_source_version,
            current_source_version,
            affected_artifact_ids,
            affected_claim_ids,
            jobs,
            outbox,
            audit,
        })
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

    fn append_model_failure_audit(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        purpose: ModelPurpose,
    ) -> Result<()> {
        let invocation = self
            .store
            .list_model_invocations(&identity.tenant_id, &atxn.atxn_id)?
            .into_iter()
            .filter(|invocation| invocation.purpose == purpose)
            .max_by_key(|invocation| invocation.attempt)
            .ok_or_else(|| {
                AmosError::Storage("failed model call has no immutable invocation record".into())
            })?;
        let action = match purpose {
            ModelPurpose::Plan => "model.plan",
            ModelPurpose::Narrative => "model.narrative",
        };
        self.store.append_audit(&AuditEvent {
            event_id: stable_id("audit", &(&atxn.atxn_id, action))?,
            tenant_id: identity.tenant_id.clone(),
            actor_id: format!("model:{}", invocation.model),
            action: action.into(),
            target_type: "model_invocation".into(),
            target_id: invocation.invocation_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: match invocation.status {
                crate::model::ModelInvocationStatus::Pass => "pass",
                crate::model::ModelInvocationStatus::Invalid => "invalid",
                crate::model::ModelInvocationStatus::Timeout => "timeout",
                crate::model::ModelInvocationStatus::ProviderError => "provider_error",
            }
            .into(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "provider": invocation.provider,
                "model": invocation.model,
                "route_class": invocation.route_class,
                "attempt": invocation.attempt,
                "input_payload_hash": invocation.input_payload_hash,
                "output_hash": invocation.output_hash,
                "error_code": invocation.error_code,
            }),
            created_at: invocation.created_at,
        })
    }

    fn append_plan_admission_audit(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        plan: &TypedPlan,
    ) -> Result<()> {
        let relations = plan
            .steps
            .iter()
            .flat_map(|step| {
                step.parameters
                    .get("relations")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
            })
            .collect::<BTreeSet<_>>();
        self.store.append_audit(&AuditEvent {
            event_id: stable_id("audit", &(&atxn.atxn_id, "plan.admit"))?,
            tenant_id: identity.tenant_id.clone(),
            actor_id: "amos:plan-admission".into(),
            action: "plan.admit".into(),
            target_type: "plan".into(),
            target_id: plan.plan_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "plan_hash": content_hash(plan)?,
                "step_count": plan.steps.len(),
                "relations": relations,
                "model_identity": plan.model_identity,
            }),
            created_at: atxn.created_at,
        })
    }

    fn append_execution_audit(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        execution: &ExecutionRecord,
    ) -> Result<()> {
        self.store.append_audit(&AuditEvent {
            event_id: stable_id(
                "audit",
                &(&atxn.atxn_id, "execution.commit", &execution.execution_id),
            )?,
            tenant_id: identity.tenant_id.clone(),
            actor_id: "amos:sql-worker".into(),
            action: "execution.commit".into(),
            target_type: "execution".into(),
            target_id: execution.execution_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: execution.status.clone(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "step_id": execution.step_id,
                "tool": execution.tool,
                "input_versions": execution.input_versions,
                "output_hash": execution.output_hash,
                "row_count": execution.row_count,
                "byte_count": execution.byte_count,
                "latency_ms": execution.latency_ms,
                "fencing_token": execution.fencing_token,
            }),
            created_at: execution.created_at,
        })
    }

    fn append_verification_audit(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        verification: &VerificationRecord,
    ) -> Result<()> {
        self.store.append_audit(&AuditEvent {
            event_id: stable_id(
                "audit",
                &(
                    &atxn.atxn_id,
                    "verification.complete",
                    &verification.verification_id,
                ),
            )?,
            tenant_id: identity.tenant_id.clone(),
            actor_id: "amos:verifier".into(),
            action: "verification.complete".into(),
            target_type: "verification".into(),
            target_id: verification.verification_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: format!("{:?}", verification.outcome).to_ascii_lowercase(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "profile": verification.verifier_profile,
                "profile_version": verification.profile_version,
                "execution_id": verification.execution_id,
                "input_hash": verification.input_hash,
                "rule_ids": verification
                    .checks
                    .iter()
                    .map(|check| check.rule_id.as_str())
                    .collect::<Vec<_>>(),
                "warning_count": verification.warnings.len(),
                "error_count": verification.errors.len(),
            }),
            created_at: verification.created_at,
        })
    }

    async fn propose_plan(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        definition: &TaskDefinition,
        manifest: &ContextManifest,
        pack: &AnalysisPack,
    ) -> Result<TypedPlan> {
        let blocked_columns = pack
            .schemas
            .iter()
            .flat_map(|schema| schema.blocked_columns.iter().cloned())
            .collect::<BTreeSet<_>>();
        let planning_objects = manifest
            .selected_objects
            .iter()
            .filter(|object| {
                matches!(
                    object.memory_type,
                    MemoryType::SemanticDefinition
                        | MemoryType::Schema
                        | MemoryType::DataState
                        | MemoryType::StreamState
                )
            })
            .collect::<Vec<_>>();
        let selected_model_object_ids = planning_objects
            .iter()
            .map(|object| object.object_id.clone())
            .collect::<Vec<_>>();
        let governed_objects = planning_objects
            .into_iter()
            .map(|object| {
                json!({
                    "object_id": object.object_id,
                    "logical_key": object.logical_key,
                    "memory_type": object.memory_type,
                    "summary": object.summary,
                    "content": sanitize_model_value(&object.content, &blocked_columns),
                    "source_id": object.source_id,
                    "source_version": object.source_version,
                    "authority": object.authority,
                    "sensitivity": object.sensitivity,
                })
            })
            .collect::<Vec<_>>();
        let (primary_relation, _) = pack.primary_relation()?;
        let primary_schema = pack
            .schemas
            .iter()
            .find(|schema| schema.relation == primary_relation)
            .ok_or_else(|| {
                AmosError::Validation("analysis pack primary schema is missing".into())
            })?;
        let query_bound = |value: &str| {
            if primary_schema.time_field.ends_with("_date") {
                value.split('T').next().unwrap_or(value).to_string()
            } else {
                value.to_string()
            }
        };
        let window_start = query_bound(&pack.time_window.start);
        let current_start = query_bound(&pack.time_window.current_start);
        let window_end = query_bound(&pack.time_window.end);
        let all_window_bounds = json!({
            "lower": format!("{} >= '{}'", primary_schema.time_field, &window_start),
            "upper": format!("{} < '{}'", primary_schema.time_field, &window_end),
        });
        let current_window_bounds = json!({
            "lower": format!("{} >= '{}'", primary_schema.time_field, &current_start),
            "upper": format!("{} < '{}'", primary_schema.time_field, &window_end),
        });
        let payload = json!({
            "planner_instructions": [
                "Return exactly one concise SQLite SELECT for each required analysis kind.",
                "Use every supplied time predicate and required metric filter verbatim as AND clauses.",
                "Use only declared relations, columns, result aliases, and literal predicates.",
                "Never repeat fragments or add prose, comments, undeclared predicates, or placeholder identifiers inside SQL.",
            ],
            "question": atxn.request,
            "task": {
                "task_type": definition.task_type,
                "version": definition.version,
                "risk_class": definition.risk_class,
                "allowed_tools": definition.allowed_tools,
                "budgets": definition.budgets,
            },
            "time_window": pack.time_window,
            "selected_governed_objects": governed_objects,
            "required_analysis_kinds": pack.required_analysis_kinds,
            "result_schemas": pack.result_schemas,
            "sql_contract": {
                "dialect": "sqlite",
                "relation": primary_relation,
                "time_field": primary_schema.time_field,
                "required_time_bounds": {
                    "rate_comparison": all_window_bounds,
                    "timeseries": all_window_bounds,
                    "concentration": current_window_bounds,
                },
                "required_metric_filters": pack.metric_required_filters,
                "required_result_semantics": {
                    "rate_comparison": {
                        "current_period_label": pack
                            .verifier_profile
                            .rate_comparison
                            .current_label,
                        "baseline_period_label": pack
                            .verifier_profile
                            .rate_comparison
                            .baseline_label,
                        "query_shape": format!(
                            "one SELECT using CASE WHEN {} >= '{}' THEN current ELSE baseline END AS {}; GROUP BY {}",
                            primary_schema.time_field,
                            &current_start,
                            pack
                                .verifier_profile
                                .rate_comparison
                                .period_field,
                            pack
                                .verifier_profile
                                .rate_comparison
                                .period_field,
                        ),
                    },
                    "concentration": {
                        "order_by": format!(
                            "{} DESC",
                            pack.verifier_profile.concentration.numerator_field
                        ),
                        "limit": 10,
                    },
                    "timeseries": {
                        "order_by": pack
                            .verifier_profile
                            .timeseries
                            .label_field,
                    },
                },
                "forbidden_sql_forms": [
                    "BETWEEN",
                    "inclusive upper bounds using <=",
                    "GROUP BY column ordinals",
                    "multiple statements",
                    "UNION, INTERSECT, or EXCEPT set operations",
                    "comments",
                    "undeclared identifiers",
                ],
            },
        });
        let output = self
            .model
            .generate_validated(
                ModelRequestTemplate {
                    tenant_id: atxn.tenant_id.clone(),
                    atxn_id: atxn.atxn_id.clone(),
                    purpose: ModelPurpose::Plan,
                    prompt_template_version: "amos.plan.prompt.v2".into(),
                    input_manifest_hash: content_hash(manifest)?,
                    payload,
                    response_schema: plan_response_schema_for_relation(primary_relation),
                    selected_object_ids: selected_model_object_ids,
                    verified_execution_ids: vec![],
                    generation_config: self.model_generation.clone(),
                },
                |proposal: &PlanProposal| {
                    validate_proposal_against_pack(proposal, pack, definition.budgets.max_steps)
                },
            )
            .await?;
        let plan = self.map_plan_proposal(
            atxn,
            definition,
            manifest,
            &output.value,
            &output.invocation,
            pack,
        )?;
        self.store.append_audit(&AuditEvent {
            event_id: stable_id("audit", &(&atxn.atxn_id, "model.plan"))?,
            tenant_id: atxn.tenant_id.clone(),
            actor_id: format!("model:{}", output.invocation.model),
            action: "model.plan".into(),
            target_type: "model_invocation".into(),
            target_id: output.invocation.invocation_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "provider": output.invocation.provider,
                "model": output.invocation.model,
                "route_class": output.invocation.route_class,
                "input_payload_hash": output.invocation.input_payload_hash,
                "output_hash": output.invocation.output_hash,
                "selected_object_count": output.invocation.selected_object_ids.len(),
            }),
            created_at: output.invocation.created_at,
        })?;
        Ok(plan)
    }

    fn map_plan_proposal(
        &self,
        atxn: &AnalyticalTransaction,
        definition: &TaskDefinition,
        manifest: &ContextManifest,
        proposal: &PlanProposal,
        invocation: &crate::model::ModelInvocation,
        pack: &AnalysisPack,
    ) -> Result<TypedPlan> {
        let input_object_ids = manifest
            .selected_objects
            .iter()
            .map(|object| object.object_id.clone())
            .collect::<Vec<_>>();
        let mut steps = Vec::with_capacity(proposal.steps.len());
        for proposed in &proposal.steps {
            let step_id = match proposed.analysis_kind {
                AnalysisKind::RateComparison => "summary",
                AnalysisKind::Concentration => "concentration",
                AnalysisKind::Timeseries => "timeseries",
            };
            let sources = proposed
                .relations
                .iter()
                .map(|relation| {
                    pack.source_relations.get(relation).cloned().ok_or_else(|| {
                        AmosError::Validation(format!(
                            "model proposed undeclared relation {relation}"
                        ))
                    })
                })
                .collect::<Result<BTreeSet<_>>>()?;
            if sources.len() != 1 {
                return Err(AmosError::Validation(
                    "one plan step cannot span multiple governed sources".into(),
                ));
            }
            let source_id = sources
                .into_iter()
                .next()
                .ok_or_else(|| AmosError::Validation("plan step has no source".into()))?;
            steps.push(PlanStep {
                step_id: step_id.into(),
                purpose: proposed.purpose.clone(),
                tool: "sql.readonly.v1".into(),
                source_id,
                input_object_ids: input_object_ids.clone(),
                parameter_schema: "amos.sql-query.v1".into(),
                parameters: json!({
                    "sql": proposed.sql,
                    "relations": proposed.relations,
                }),
                expected_output_schema: serde_json::to_string(&proposed.expected_columns)?,
                limits: crate::domain::OperationLimits {
                    seconds: definition.budgets.max_seconds,
                    rows: definition.budgets.max_rows,
                    bytes: definition.budgets.max_bytes,
                },
                max_attempts: definition.budgets.max_repairs.saturating_add(1),
                repair_classes: if definition.budgets.max_repairs > 0 {
                    BTreeSet::from(["COLUMN_SUPERSEDED".into()])
                } else {
                    BTreeSet::new()
                },
                verifier_profile: definition.verifier_profile.clone(),
            });
        }
        Ok(TypedPlan {
            plan_id: stable_id(
                "plan",
                &(
                    &atxn.tenant_id,
                    &atxn.atxn_id,
                    &invocation.invocation_id,
                    proposal,
                ),
            )?,
            tenant_id: atxn.tenant_id.clone(),
            atxn_id: atxn.atxn_id.clone(),
            task_definition: manifest.task_definition.clone(),
            manifest_id: manifest.manifest_id.clone(),
            model_identity: format!("{}:{}", invocation.provider, invocation.model),
            steps,
        })
    }

    async fn generate_narrative(
        &self,
        identity: &Identity,
        atxn: &AnalyticalTransaction,
        manifest: &ContextManifest,
        catalog: &VerifiedFactCatalog,
        pack: &AnalysisPack,
    ) -> Result<NarrativePlan> {
        let narrative_objects = manifest
            .selected_objects
            .iter()
            .filter(|object| {
                matches!(
                    object.memory_type,
                    MemoryType::DataState
                        | MemoryType::StreamState
                        | MemoryType::Document
                        | MemoryType::PriorAnalysis
                        | MemoryType::Feedback
                        | MemoryType::ReviewPolicy
                )
            })
            .collect::<Vec<_>>();
        let permitted_context = narrative_objects
            .iter()
            .map(|object| {
                json!({
                    "object_id":object.object_id,
                    "logical_key":object.logical_key,
                    "memory_type":object.memory_type,
                    "summary":object.summary,
                })
            })
            .collect::<Vec<_>>();
        let permitted_memory_ids = narrative_objects
            .iter()
            .map(|object| object.object_id.clone())
            .collect::<BTreeSet<_>>();
        let fact_ids = catalog
            .facts
            .iter()
            .map(|fact| fact.fact_id.clone())
            .collect::<BTreeSet<_>>();
        let narrative_facts = catalog
            .facts
            .iter()
            .map(|fact| {
                let qualitative_hint = match fact.claim_type.as_str() {
                    "metric_comparison" => {
                        "The governed metric changed between the baseline and current periods."
                    }
                    "concentration" => {
                        "The governed result identifies the largest segment concentration."
                    }
                    "timeseries" => {
                        "The governed result contains a daily trend with a freshness caveat."
                    }
                    _ => "A governed verified fact is available.",
                };
                json!({
                    "fact_id": fact.fact_id,
                    "claim_type": fact.claim_type,
                    "qualitative_hint": qualitative_hint,
                    "render_placeholder": format!("{{{{fact:{}}}}}", fact.fact_id),
                    "freshness_labels": fact.freshness_labels,
                    "governed_memory_ids": fact.governed_memory_ids,
                })
            })
            .collect::<Vec<_>>();
        let response_schema = narrative_response_schema_for_evidence(
            &fact_ids,
            &permitted_memory_ids,
            &pack.review_triggering_claim_types,
        );
        let verified_execution_ids = catalog
            .facts
            .iter()
            .flat_map(|fact| fact.supporting_execution_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let output = self
            .model
            .generate_validated(
                ModelRequestTemplate {
                    tenant_id: atxn.tenant_id.clone(),
                    atxn_id: atxn.atxn_id.clone(),
                    purpose: ModelPurpose::Narrative,
                    prompt_template_version: "amos.narrative.prompt.v2".into(),
                    input_manifest_hash: catalog.content_hash.clone(),
                    payload: json!({
                        "narrative_instructions": [
                            "Use render_placeholder values instead of copying numeric literals into model-authored text.",
                            "Reference every verified fact exactly once in finding_order.",
                            "Return exactly one separately reviewable judgment for each required_judgment_claim.",
                            "Use only the supplied fact IDs and governed memory IDs.",
                        ],
                        "verified_fact_catalog": {
                            "content_hash": catalog.content_hash,
                            "facts": narrative_facts,
                        },
                        "permitted_context":permitted_context,
                        "permitted_memory_ids":permitted_memory_ids,
                        "required_judgment_claims": pack
                            .review_triggering_claim_types
                            .iter()
                            .map(|claim_type| json!({
                                "claim_type": claim_type,
                                "review_required": true,
                            }))
                            .collect::<Vec<_>>(),
                        "review_triggering_claim_types":
                            pack.review_triggering_claim_types,
                    }),
                    response_schema,
                    selected_object_ids: permitted_memory_ids.iter().cloned().collect(),
                    verified_execution_ids,
                    generation_config: self.model_generation.clone(),
                },
                |narrative: &NarrativePlan| {
                    validate_narrative_plan(
                        narrative,
                        catalog,
                        &permitted_memory_ids,
                        pack,
                    )
                },
            )
            .await?;
        self.store.append_audit(&AuditEvent {
            event_id: stable_id("audit", &(&atxn.atxn_id, "model.narrative"))?,
            tenant_id: atxn.tenant_id.clone(),
            actor_id: format!("model:{}", output.invocation.model),
            action: "model.narrative".into(),
            target_type: "model_invocation".into(),
            target_id: output.invocation.invocation_id.clone(),
            request_id: Some(atxn.request_id.clone()),
            atxn_id: Some(atxn.atxn_id.clone()),
            outcome: "pass".into(),
            policy_epoch: identity.policy_epoch,
            details: json!({
                "provider":output.invocation.provider,
                "model":output.invocation.model,
                "route_class":output.invocation.route_class,
                "input_payload_hash":output.invocation.input_payload_hash,
                "output_hash":output.invocation.output_hash,
                "verified_execution_count":output.invocation.verified_execution_ids.len(),
            }),
            created_at: output.invocation.created_at,
        })?;
        Ok(output.value)
    }

    fn replay_package(
        &self,
        artifact: &Artifact,
        manifest: &ContextManifest,
        plan: &TypedPlan,
        executions: &[ExecutionRecord],
        pack: &AnalysisPack,
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
            template: pack.report_template.clone(),
            render_config_hash: content_hash(&pack.report_template)?,
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

fn validate_proposal_against_pack(
    proposal: &PlanProposal,
    pack: &AnalysisPack,
    max_steps: u32,
) -> Result<()> {
    proposal.validate(max_steps)?;
    let proposed_kinds = proposal
        .steps
        .iter()
        .map(|step| step.analysis_kind)
        .collect::<BTreeSet<_>>();
    if proposed_kinds != pack.required_analysis_kinds {
        return Err(AmosError::Validation(
            "model plan does not contain the exact required analysis kinds".into(),
        ));
    }
    let allowed_relations = pack.source_relations.keys().collect::<BTreeSet<_>>();
    for step in &proposal.steps {
        if !step
            .relations
            .iter()
            .all(|relation| allowed_relations.contains(relation))
        {
            return Err(AmosError::Validation(
                "model plan requested a relation outside the pack allowlist".into(),
            ));
        }
        let expected = pack
            .result_schemas
            .get(&step.analysis_kind)
            .ok_or_else(|| AmosError::Validation("analysis result schema is missing".into()))?;
        if &step.expected_columns != expected {
            return Err(AmosError::Validation(
                "model plan output shape does not match the pack".into(),
            ));
        }
    }
    Ok(())
}

fn sanitize_model_value(value: &Value, blocked_columns: &BTreeSet<String>) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| !blocked_columns.contains(key.as_str()))
                .map(|(key, value)| (key.clone(), sanitize_model_value(value, blocked_columns)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| sanitize_model_value(value, blocked_columns))
                .collect(),
        ),
        _ => value.clone(),
    }
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
