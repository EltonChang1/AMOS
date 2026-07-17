use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    Result,
    domain::{
        AnalyticalTransaction, Artifact, AtxnState, AuditEvent, Claim, ContextManifest,
        DependencyEdge, ExecutionRecord, Job, JobState, MemoryObject, MemoryStatus, OutboxEvent,
        Outcome, PublicationValidity, ReplayPackage, Review, TaskDefinition, TypedPlan,
        VerificationRecord,
    },
    error::AmosError,
};

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| AmosError::Storage(error.to_string()))?;
        }
        let connection = Connection::open(path)?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        let store = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| AmosError::Storage("database lock poisoned".into()))
    }

    fn init_schema(&self) -> Result<()> {
        let connection = self.connection()?;
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS memory_objects (
                tenant_id TEXT NOT NULL,
                object_id TEXT NOT NULL,
                logical_key TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                source_id TEXT NOT NULL,
                source_version TEXT NOT NULL,
                authority INTEGER NOT NULL,
                effective_start TEXT,
                effective_end TEXT,
                recorded_at TEXT NOT NULL,
                permissions_json TEXT NOT NULL,
                sensitivity TEXT NOT NULL,
                version TEXT NOT NULL,
                status TEXT NOT NULL,
                superseded_by TEXT,
                content_hash TEXT NOT NULL,
                governing INTEGER NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, object_id),
                UNIQUE (tenant_id, source_id, logical_key, source_version)
            );
            CREATE INDEX IF NOT EXISTS idx_memory_scope
                ON memory_objects(tenant_id, memory_type, status, logical_key);
            CREATE INDEX IF NOT EXISTS idx_memory_effective
                ON memory_objects(tenant_id, effective_start, effective_end);
            CREATE INDEX IF NOT EXISTS idx_memory_source_version
                ON memory_objects(tenant_id, source_id, source_version);
            CREATE INDEX IF NOT EXISTS idx_memory_superseded
                ON memory_objects(tenant_id, superseded_by);

            CREATE TABLE IF NOT EXISTS task_definitions (
                tenant_id TEXT NOT NULL,
                task_type TEXT NOT NULL,
                version INTEGER NOT NULL,
                status TEXT NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, task_type, version)
            );

            CREATE TABLE IF NOT EXISTS atxn_transactions (
                tenant_id TEXT NOT NULL,
                atxn_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                state TEXT NOT NULL,
                state_seq INTEGER NOT NULL,
                terminal INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, atxn_id),
                UNIQUE (tenant_id, idempotency_key)
            );
            CREATE INDEX IF NOT EXISTS idx_atxn_state
                ON atxn_transactions(tenant_id, state, updated_at);

            CREATE TABLE IF NOT EXISTS context_manifests (
                tenant_id TEXT NOT NULL, manifest_id TEXT NOT NULL, atxn_id TEXT NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, manifest_id)
            );
            CREATE TABLE IF NOT EXISTS plans (
                tenant_id TEXT NOT NULL, plan_id TEXT NOT NULL, atxn_id TEXT NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, plan_id)
            );
            CREATE TABLE IF NOT EXISTS executions (
                tenant_id TEXT NOT NULL, execution_id TEXT NOT NULL, atxn_id TEXT NOT NULL,
                step_id TEXT NOT NULL, output_hash TEXT NOT NULL, fencing_token INTEGER NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, execution_id)
            );
            CREATE TABLE IF NOT EXISTS verification_records (
                tenant_id TEXT NOT NULL, verification_id TEXT NOT NULL, atxn_id TEXT NOT NULL,
                outcome TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, verification_id)
            );
            CREATE TABLE IF NOT EXISTS artifacts (
                tenant_id TEXT NOT NULL, artifact_id TEXT NOT NULL, atxn_id TEXT NOT NULL,
                content_hash TEXT NOT NULL, publication_validity TEXT NOT NULL,
                validity_seq INTEGER NOT NULL DEFAULT 0, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, artifact_id)
            );
            CREATE TABLE IF NOT EXISTS claims (
                tenant_id TEXT NOT NULL, claim_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                semantic_validity TEXT NOT NULL, policy_visibility TEXT NOT NULL,
                review_state TEXT NOT NULL, supersession_state TEXT NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, claim_id)
            );
            CREATE INDEX IF NOT EXISTS idx_claim_artifact ON claims(tenant_id, artifact_id);
            CREATE TABLE IF NOT EXISTS dependency_edges (
                tenant_id TEXT NOT NULL, edge_id TEXT NOT NULL,
                from_type TEXT NOT NULL, from_id TEXT NOT NULL,
                relation TEXT NOT NULL, to_type TEXT NOT NULL, to_id TEXT NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, edge_id)
            );
            CREATE INDEX IF NOT EXISTS idx_edge_target
                ON dependency_edges(tenant_id, to_type, to_id);
            CREATE INDEX IF NOT EXISTS idx_edge_source
                ON dependency_edges(tenant_id, from_type, from_id);
            CREATE TABLE IF NOT EXISTS replay_packages (
                tenant_id TEXT NOT NULL, package_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                retained_until TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, package_id), UNIQUE (tenant_id, artifact_id)
            );
            CREATE TABLE IF NOT EXISTS reviews (
                tenant_id TEXT NOT NULL, review_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                reviewer_id TEXT NOT NULL, decision TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, review_id)
            );
            CREATE TABLE IF NOT EXISTS audit_events (
                tenant_id TEXT NOT NULL, event_id TEXT NOT NULL, actor_id TEXT NOT NULL,
                action TEXT NOT NULL, target_type TEXT NOT NULL, target_id TEXT NOT NULL,
                created_at TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, event_id)
            );
            CREATE INDEX IF NOT EXISTS idx_audit_created
                ON audit_events(tenant_id, created_at DESC);
            CREATE TABLE IF NOT EXISTS outbox_events (
                tenant_id TEXT NOT NULL, event_id TEXT NOT NULL, event_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL, idempotency_key TEXT NOT NULL,
                payload_json TEXT NOT NULL, created_at TEXT NOT NULL, completed_at TEXT,
                PRIMARY KEY (tenant_id, event_id), UNIQUE (tenant_id, idempotency_key)
            );
            CREATE TABLE IF NOT EXISTS jobs (
                tenant_id TEXT NOT NULL, job_id TEXT NOT NULL, job_type TEXT NOT NULL,
                idempotency_key TEXT NOT NULL, state TEXT NOT NULL, fencing_token INTEGER NOT NULL,
                lease_owner TEXT, lease_expires_at TEXT, next_run_at TEXT NOT NULL,
                body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, job_id),
                UNIQUE (tenant_id, idempotency_key)
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_ready
                ON jobs(tenant_id, state, next_run_at, lease_expires_at);
            "#,
        )?;
        Ok(())
    }

    pub fn write_memory(&self, object: &MemoryObject) -> Result<()> {
        let body = to_json(object)?;
        let permissions = to_json(&object.permissions)?;
        let connection = self.connection()?;
        let existing_hash: Option<String> = connection
            .query_row(
                "SELECT content_hash FROM memory_objects WHERE tenant_id=?1 AND source_id=?2 AND logical_key=?3 AND source_version=?4",
                params![object.tenant_id, object.source_id, object.logical_key, object.source_version],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(hash) = existing_hash {
            return if hash == object.content_hash {
                Ok(())
            } else {
                Err(AmosError::Conflict(format!(
                    "source version {} changed content for {}",
                    object.source_version, object.logical_key
                )))
            };
        }
        connection.execute(
            r#"INSERT INTO memory_objects
            (tenant_id, object_id, logical_key, memory_type, source_id, source_version,
             authority, effective_start, effective_end, recorded_at, permissions_json,
             sensitivity, version, status, superseded_by, content_hash, governing, body_json)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)"#,
            params![
                object.tenant_id,
                object.object_id,
                object.logical_key,
                enum_json(&object.memory_type)?,
                object.source_id,
                object.source_version,
                object.authority.rank(),
                object.effective_start.map(|value| value.to_rfc3339()),
                object.effective_end.map(|value| value.to_rfc3339()),
                object.recorded_at.to_rfc3339(),
                permissions,
                object.sensitivity,
                object.version,
                enum_json(&object.status)?,
                object.superseded_by,
                object.content_hash,
                object.governing,
                body,
            ],
        )?;
        Ok(())
    }

    pub fn get_memory(&self, tenant_id: &str, object_id: &str) -> Result<Option<MemoryObject>> {
        self.get_json(
            "SELECT body_json FROM memory_objects WHERE tenant_id=?1 AND object_id=?2",
            params![tenant_id, object_id],
        )
    }

    pub fn update_memory(&self, object: &MemoryObject) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            r#"UPDATE memory_objects SET
                logical_key=?1, memory_type=?2, source_id=?3, source_version=?4,
                authority=?5, effective_start=?6, effective_end=?7, recorded_at=?8,
                permissions_json=?9, sensitivity=?10, version=?11, status=?12,
                superseded_by=?13, content_hash=?14, governing=?15, body_json=?16
             WHERE tenant_id=?17 AND object_id=?18"#,
            params![
                object.logical_key,
                enum_json(&object.memory_type)?,
                object.source_id,
                object.source_version,
                object.authority.rank(),
                object.effective_start.map(|value| value.to_rfc3339()),
                object.effective_end.map(|value| value.to_rfc3339()),
                object.recorded_at.to_rfc3339(),
                to_json(&object.permissions)?,
                object.sensitivity,
                object.version,
                enum_json(&object.status)?,
                object.superseded_by,
                object.content_hash,
                object.governing,
                to_json(object)?,
                object.tenant_id,
                object.object_id,
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::NotFound(object.object_id.clone()));
        }
        Ok(())
    }

    pub fn list_active_memory(&self, tenant_id: &str) -> Result<Vec<MemoryObject>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT body_json FROM memory_objects WHERE tenant_id=?1 AND status='active' AND superseded_by IS NULL",
        )?;
        let rows = statement.query_map([tenant_id], |row| row.get::<_, String>(0))?;
        rows.map(|row| from_json(&row?)).collect()
    }

    pub fn supersede_memory(
        &self,
        tenant_id: &str,
        old_id: &str,
        new_object: &MemoryObject,
    ) -> Result<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old_body: String = transaction.query_row(
            "SELECT body_json FROM memory_objects WHERE tenant_id=?1 AND object_id=?2",
            params![tenant_id, old_id],
            |row| row.get(0),
        )?;
        let mut old: MemoryObject = from_json(&old_body)?;
        if old.status != MemoryStatus::Active {
            return Err(AmosError::Conflict(format!(
                "memory {old_id} is not active"
            )));
        }
        old.status = MemoryStatus::Superseded;
        old.superseded_by = Some(new_object.object_id.clone());
        transaction.execute(
            "UPDATE memory_objects SET status='superseded', superseded_by=?1, body_json=?2 WHERE tenant_id=?3 AND object_id=?4 AND status='active'",
            params![new_object.object_id, to_json(&old)?, tenant_id, old_id],
        )?;
        insert_memory_tx(&transaction, new_object)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn put_task_definition(&self, definition: &TaskDefinition) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO task_definitions (tenant_id,task_type,version,status,body_json) VALUES (?1,?2,?3,?4,?5)",
            params![definition.tenant_id, definition.task_type, definition.version, definition.status, to_json(definition)?],
        )?;
        Ok(())
    }

    pub fn get_task_definition(
        &self,
        tenant: &str,
        task_type: &str,
    ) -> Result<Option<TaskDefinition>> {
        self.get_json(
            "SELECT body_json FROM task_definitions WHERE tenant_id=?1 AND task_type=?2 AND status='approved' ORDER BY version DESC LIMIT 1",
            params![tenant, task_type],
        )
    }

    pub fn create_transaction(
        &self,
        transaction: &AnalyticalTransaction,
    ) -> Result<AnalyticalTransaction> {
        let connection = self.connection()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND idempotency_key=?2",
                params![transaction.tenant_id, transaction.idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: AnalyticalTransaction = from_json(&existing)?;
            return if existing.request_hash == transaction.request_hash {
                Ok(existing)
            } else {
                Err(AmosError::IdempotencyConflict(
                    transaction.idempotency_key.clone(),
                ))
            };
        }
        connection.execute(
            "INSERT INTO atxn_transactions (tenant_id,atxn_id,idempotency_key,request_hash,state,state_seq,terminal,updated_at,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![transaction.tenant_id, transaction.atxn_id, transaction.idempotency_key, transaction.request_hash, enum_json(&transaction.state)?, transaction.state_seq as i64, transaction.terminal, transaction.updated_at.to_rfc3339(), to_json(transaction)?],
        )?;
        Ok(transaction.clone())
    }

    pub fn get_transaction(
        &self,
        tenant: &str,
        atxn_id: &str,
    ) -> Result<Option<AnalyticalTransaction>> {
        self.get_json(
            "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND atxn_id=?2",
            params![tenant, atxn_id],
        )
    }

    pub fn transition_transaction(
        &self,
        tenant: &str,
        atxn_id: &str,
        expected: AtxnState,
        expected_seq: u64,
        next: AtxnState,
        outcome: Option<crate::domain::Outcome>,
    ) -> Result<AnalyticalTransaction> {
        if !expected.can_transition(next) {
            return Err(AmosError::InvalidTransition(format!(
                "{expected:?} -> {next:?}"
            )));
        }
        let mut transaction = self
            .get_transaction(tenant, atxn_id)?
            .ok_or_else(|| AmosError::NotFound(atxn_id.into()))?;
        if transaction.state != expected
            || transaction.state_seq != expected_seq
            || transaction.terminal
        {
            return Err(AmosError::Conflict(format!(
                "stale transaction sequence for {atxn_id}"
            )));
        }
        transaction.state = next;
        transaction.state_seq += 1;
        transaction.updated_at = Utc::now();
        transaction.terminal = next.terminal();
        if outcome.is_some() {
            transaction.outcome = outcome;
        }
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE atxn_transactions SET state=?1,state_seq=?2,terminal=?3,updated_at=?4,body_json=?5 WHERE tenant_id=?6 AND atxn_id=?7 AND state=?8 AND state_seq=?9 AND terminal=0",
            params![enum_json(&next)?, transaction.state_seq as i64, transaction.terminal, transaction.updated_at.to_rfc3339(), to_json(&transaction)?, tenant, atxn_id, enum_json(&expected)?, expected_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(format!(
                "compare-and-swap failed for {atxn_id}"
            )));
        }
        Ok(transaction)
    }

    pub fn checkpoint_transaction(&self, transaction: &AnalyticalTransaction) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE atxn_transactions SET updated_at=?1,body_json=?2 WHERE tenant_id=?3 AND atxn_id=?4 AND state=?5 AND state_seq=?6",
            params![transaction.updated_at.to_rfc3339(),to_json(transaction)?,transaction.tenant_id,transaction.atxn_id,enum_json(&transaction.state)?,transaction.state_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "transaction checkpoint lost compare-and-swap".into(),
            ));
        }
        Ok(())
    }

    pub fn save_manifest(&self, manifest: &ContextManifest) -> Result<()> {
        self.insert_json(
            "context_manifests",
            &manifest.tenant_id,
            "manifest_id",
            &manifest.manifest_id,
            Some(("atxn_id", &manifest.atxn_id)),
            manifest,
        )
    }

    pub fn get_manifest(&self, tenant: &str, id: &str) -> Result<Option<ContextManifest>> {
        self.get_json(
            "SELECT body_json FROM context_manifests WHERE tenant_id=?1 AND manifest_id=?2",
            params![tenant, id],
        )
    }

    pub fn save_plan(&self, plan: &TypedPlan) -> Result<()> {
        self.insert_json(
            "plans",
            &plan.tenant_id,
            "plan_id",
            &plan.plan_id,
            Some(("atxn_id", &plan.atxn_id)),
            plan,
        )
    }

    pub fn get_plan(&self, tenant: &str, id: &str) -> Result<Option<TypedPlan>> {
        self.get_json(
            "SELECT body_json FROM plans WHERE tenant_id=?1 AND plan_id=?2",
            params![tenant, id],
        )
    }

    pub fn save_execution(&self, execution: &ExecutionRecord) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO executions (tenant_id,execution_id,atxn_id,step_id,output_hash,fencing_token,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![execution.tenant_id,execution.execution_id,execution.atxn_id,execution.step_id,execution.output_hash,execution.fencing_token as i64,to_json(execution)?],
        )?;
        Ok(())
    }

    pub fn list_executions(&self, tenant: &str, atxn_id: &str) -> Result<Vec<ExecutionRecord>> {
        self.list_json(
            "SELECT body_json FROM executions WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid",
            params![tenant, atxn_id],
        )
    }

    pub fn save_verification(&self, verification: &VerificationRecord) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO verification_records (tenant_id,verification_id,atxn_id,outcome,body_json) VALUES (?1,?2,?3,?4,?5)",
            params![verification.tenant_id,verification.verification_id,verification.atxn_id,enum_json(&verification.outcome)?,to_json(verification)?],
        )?;
        Ok(())
    }

    pub fn list_verifications(
        &self,
        tenant: &str,
        atxn_id: &str,
    ) -> Result<Vec<VerificationRecord>> {
        self.list_json("SELECT body_json FROM verification_records WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid", params![tenant,atxn_id])
    }

    pub fn commit_evidence(
        &self,
        atxn: &AnalyticalTransaction,
        artifact: &Artifact,
        claims: &[Claim],
        edges: &[DependencyEdge],
        package: &ReplayPackage,
        audit: &AuditEvent,
    ) -> Result<AnalyticalTransaction> {
        if atxn.state != AtxnState::Revalidating
            || !atxn.state.can_transition(AtxnState::EvidenceCommitted)
        {
            return Err(AmosError::InvalidTransition(
                "evidence commit requires revalidating state".into(),
            ));
        }
        let mut committed = atxn.clone();
        committed.state = AtxnState::EvidenceCommitted;
        committed.state_seq += 1;
        committed.updated_at = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE atxn_transactions SET state='evidence_committed',state_seq=?1,updated_at=?2,body_json=?3 WHERE tenant_id=?4 AND atxn_id=?5 AND state='revalidating' AND state_seq=?6 AND terminal=0",
            params![committed.state_seq as i64,committed.updated_at.to_rfc3339(),to_json(&committed)?,atxn.tenant_id,atxn.atxn_id,atxn.state_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "evidence commit lost transaction compare-and-swap".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO artifacts (tenant_id,artifact_id,atxn_id,content_hash,publication_validity,body_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![artifact.tenant_id,artifact.artifact_id,artifact.atxn_id,artifact.content_hash,enum_json(&artifact.publication_validity)?,to_json(artifact)?],
        )?;
        for claim in claims {
            transaction.execute(
                "INSERT INTO claims (tenant_id,claim_id,artifact_id,semantic_validity,policy_visibility,review_state,supersession_state,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![claim.tenant_id,claim.claim_id,claim.artifact_id,enum_json(&claim.semantic_validity)?,enum_json(&claim.policy_visibility)?,enum_json(&claim.review_state)?,enum_json(&claim.supersession_state)?,to_json(claim)?],
            )?;
        }
        for edge in edges {
            transaction.execute(
                "INSERT INTO dependency_edges (tenant_id,edge_id,from_type,from_id,relation,to_type,to_id,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![edge.tenant_id,edge.edge_id,edge.from.endpoint_type,edge.from.id,edge.relation,edge.to.endpoint_type,edge.to.id,to_json(edge)?],
            )?;
        }
        transaction.execute(
            "INSERT INTO replay_packages (tenant_id,package_id,artifact_id,retained_until,body_json) VALUES (?1,?2,?3,?4,?5)",
            params![package.tenant_id,package.package_id,package.artifact_id,package.retained_until.to_rfc3339(),to_json(package)?],
        )?;
        insert_audit_tx(&transaction, audit)?;
        transaction.execute(
            "INSERT INTO outbox_events (tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,created_at) VALUES (?1,?2,'evidence.committed',?3,?4,?5,?6)",
            params![artifact.tenant_id, crate::domain::new_id("evt"), artifact.artifact_id, format!("{}/evidence",artifact.artifact_id), to_json(&serde_json::json!({"artifact_id":artifact.artifact_id}))?, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(committed)
    }

    pub fn get_artifact(&self, tenant: &str, id: &str) -> Result<Option<Artifact>> {
        self.get_json(
            "SELECT body_json FROM artifacts WHERE tenant_id=?1 AND artifact_id=?2",
            params![tenant, id],
        )
    }

    pub fn get_artifact_by_atxn(&self, tenant: &str, atxn_id: &str) -> Result<Option<Artifact>> {
        self.get_json("SELECT body_json FROM artifacts WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid DESC LIMIT 1", params![tenant,atxn_id])
    }

    pub fn list_artifacts(&self, tenant: &str, limit: usize) -> Result<Vec<Artifact>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT body_json FROM artifacts WHERE tenant_id=?1 ORDER BY rowid DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![tenant, limit.min(100) as i64], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| from_json(&row?)).collect()
    }

    pub fn commit_local_publication(
        &self,
        atxn: &AnalyticalTransaction,
        artifact: &mut Artifact,
        claims: &mut [Claim],
        outcome: Outcome,
        audit: &AuditEvent,
    ) -> Result<AnalyticalTransaction> {
        if atxn.state != AtxnState::PublicationPending
            || !atxn.state.can_transition(AtxnState::Published)
        {
            return Err(AmosError::InvalidTransition(
                "local publication requires publication_pending state".into(),
            ));
        }
        let mut published = atxn.clone();
        published.state = AtxnState::Published;
        published.state_seq += 1;
        published.updated_at = Utc::now();
        published.outcome = Some(outcome);
        artifact.publication_validity = PublicationValidity::ValidAtPublication;
        for claim in claims.iter_mut() {
            claim.publication_validity = PublicationValidity::ValidAtPublication;
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE atxn_transactions SET state='published',state_seq=?1,updated_at=?2,body_json=?3 WHERE tenant_id=?4 AND atxn_id=?5 AND state='publication_pending' AND state_seq=?6 AND terminal=0",
            params![published.state_seq as i64,published.updated_at.to_rfc3339(),to_json(&published)?,atxn.tenant_id,atxn.atxn_id,atxn.state_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "local publication lost transaction compare-and-swap".into(),
            ));
        }
        let changed = transaction.execute(
            "UPDATE artifacts SET publication_validity='valid_at_publication',validity_seq=validity_seq+1,body_json=?1 WHERE tenant_id=?2 AND artifact_id=?3 AND publication_validity='draft'",
            params![to_json(artifact)?,artifact.tenant_id,artifact.artifact_id],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "artifact publication validity changed concurrently".into(),
            ));
        }
        for claim in claims.iter() {
            transaction.execute(
                "UPDATE claims SET body_json=?1 WHERE tenant_id=?2 AND claim_id=?3",
                params![to_json(claim)?, claim.tenant_id, claim.claim_id],
            )?;
        }
        insert_audit_tx(&transaction, audit)?;
        transaction.execute(
            "INSERT INTO outbox_events (tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,created_at) VALUES (?1,?2,'artifact.published',?3,?4,?5,?6)",
            params![artifact.tenant_id,crate::domain::new_id("evt"),artifact.artifact_id,format!("{}/published",artifact.artifact_id),to_json(&serde_json::json!({"artifact_id":artifact.artifact_id}))?,Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(published)
    }

    pub fn list_claims(&self, tenant: &str, artifact: &str) -> Result<Vec<Claim>> {
        self.list_json(
            "SELECT body_json FROM claims WHERE tenant_id=?1 AND artifact_id=?2 ORDER BY rowid",
            params![tenant, artifact],
        )
    }

    pub fn get_claim(&self, tenant: &str, claim_id: &str) -> Result<Option<Claim>> {
        self.get_json(
            "SELECT body_json FROM claims WHERE tenant_id=?1 AND claim_id=?2",
            params![tenant, claim_id],
        )
    }

    pub fn list_edges_from(
        &self,
        tenant: &str,
        source_type: &str,
        source_id: &str,
    ) -> Result<Vec<DependencyEdge>> {
        self.list_json("SELECT body_json FROM dependency_edges WHERE tenant_id=?1 AND from_type=?2 AND from_id=?3", params![tenant,source_type,source_id])
    }

    pub fn list_edges_to(
        &self,
        tenant: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<Vec<DependencyEdge>> {
        self.list_json(
            "SELECT body_json FROM dependency_edges WHERE tenant_id=?1 AND to_type=?2 AND to_id=?3",
            params![tenant, target_type, target_id],
        )
    }

    pub fn get_replay_package(
        &self,
        tenant: &str,
        artifact: &str,
    ) -> Result<Option<ReplayPackage>> {
        self.get_json(
            "SELECT body_json FROM replay_packages WHERE tenant_id=?1 AND artifact_id=?2",
            params![tenant, artifact],
        )
    }

    pub fn save_review(&self, review: &Review) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO reviews (tenant_id,review_id,artifact_id,reviewer_id,decision,body_json) VALUES (?1,?2,?3,?4,?5,?6)",
            params![review.tenant_id,review.review_id,review.artifact_id,review.reviewer_id,enum_json(&review.decision)?,to_json(review)?],
        )?;
        Ok(())
    }

    pub fn update_claim(&self, claim: &Claim) -> Result<()> {
        let connection = self.connection()?;
        connection.execute(
            "UPDATE claims SET semantic_validity=?1,policy_visibility=?2,review_state=?3,supersession_state=?4,body_json=?5 WHERE tenant_id=?6 AND claim_id=?7",
            params![enum_json(&claim.semantic_validity)?,enum_json(&claim.policy_visibility)?,enum_json(&claim.review_state)?,enum_json(&claim.supersession_state)?,to_json(claim)?,claim.tenant_id,claim.claim_id],
        )?;
        Ok(())
    }

    pub fn update_artifact(&self, artifact: &Artifact) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE artifacts SET content_hash=?1,publication_validity=?2,body_json=?3 WHERE tenant_id=?4 AND artifact_id=?5",
            params![
                artifact.content_hash,
                enum_json(&artifact.publication_validity)?,
                to_json(artifact)?,
                artifact.tenant_id,
                artifact.artifact_id
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::NotFound(artifact.artifact_id.clone()));
        }
        Ok(())
    }

    pub fn list_pending_outbox(&self, tenant: &str, limit: usize) -> Result<Vec<OutboxEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,created_at,completed_at FROM outbox_events WHERE tenant_id=?1 AND completed_at IS NULL ORDER BY created_at ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![tenant, limit.min(250) as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (
                tenant_id,
                event_id,
                event_type,
                aggregate_id,
                idempotency_key,
                payload,
                created_at,
                completed_at,
            ) = row?;
            Ok(OutboxEvent {
                tenant_id,
                event_id,
                event_type,
                aggregate_id,
                idempotency_key,
                payload: serde_json::from_str(&payload)?,
                created_at: parse_time(&created_at)?,
                completed_at: completed_at.as_deref().map(parse_time).transpose()?,
            })
        })
        .collect()
    }

    pub fn complete_outbox(&self, tenant: &str, event_id: &str) -> Result<OutboxEvent> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "SELECT tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,created_at,completed_at FROM outbox_events WHERE tenant_id=?1 AND event_id=?2",
                params![tenant, event_id],
                |row| {
                    Ok(OutboxRow {
                        tenant_id: row.get(0)?,
                        event_id: row.get(1)?,
                        event_type: row.get(2)?,
                        aggregate_id: row.get(3)?,
                        idempotency_key: row.get(4)?,
                        payload: row.get(5)?,
                        created_at: row.get(6)?,
                        completed_at: row.get(7)?,
                    })
                },
            )
            .optional()?;
        let Some(OutboxRow {
            tenant_id,
            event_id: stored_event_id,
            event_type,
            aggregate_id,
            idempotency_key,
            payload,
            created_at,
            completed_at,
        }) = row
        else {
            return Err(AmosError::NotFound(event_id.into()));
        };
        let completed = if let Some(existing) = completed_at {
            parse_time(&existing)?
        } else {
            let now = Utc::now();
            let changed = transaction.execute(
                "UPDATE outbox_events SET completed_at=?1 WHERE tenant_id=?2 AND event_id=?3 AND completed_at IS NULL",
                params![now.to_rfc3339(), tenant, stored_event_id],
            )?;
            if changed != 1 {
                return Err(AmosError::Conflict(
                    "outbox event completed concurrently".into(),
                ));
            }
            now
        };
        transaction.commit()?;
        Ok(OutboxEvent {
            tenant_id,
            event_id: stored_event_id,
            event_type,
            aggregate_id,
            idempotency_key,
            payload: serde_json::from_str(&payload)?,
            created_at: parse_time(&created_at)?,
            completed_at: Some(completed),
        })
    }

    pub fn append_audit(&self, event: &AuditEvent) -> Result<()> {
        let connection = self.connection()?;
        insert_audit_tx(&connection, event)
    }

    pub fn list_audit(&self, tenant: &str, limit: usize) -> Result<Vec<AuditEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT body_json FROM audit_events WHERE tenant_id=?1 ORDER BY created_at DESC LIMIT ?2")?;
        let rows = statement.query_map(params![tenant, limit.min(250) as i64], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| from_json(&row?)).collect()
    }

    pub fn enqueue_job(&self, job: &Job) -> Result<Job> {
        let connection = self.connection()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT body_json FROM jobs WHERE tenant_id=?1 AND idempotency_key=?2",
                params![job.tenant_id, job.idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return from_json(&existing);
        }
        connection.execute(
            "INSERT INTO jobs (tenant_id,job_id,job_type,idempotency_key,state,fencing_token,lease_owner,lease_expires_at,next_run_at,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![job.tenant_id,job.job_id,job.job_type,job.idempotency_key,enum_json(&job.state)?,job.fencing_token as i64,job.lease_owner,job.lease_expires_at.map(|v|v.to_rfc3339()),job.next_run_at.to_rfc3339(),to_json(job)?],
        )?;
        Ok(job.clone())
    }

    pub fn acquire_job(
        &self,
        tenant: &str,
        worker: &str,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<Job>> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let body: Option<String> = transaction.query_row(
            "SELECT body_json FROM jobs WHERE tenant_id=?1 AND state IN ('ready','retry_wait') AND next_run_at<=?2 AND (lease_expires_at IS NULL OR lease_expires_at<?2) ORDER BY next_run_at LIMIT 1",
            params![tenant,Utc::now().to_rfc3339()], |row| row.get(0)
        ).optional()?;
        let Some(body) = body else {
            transaction.commit()?;
            return Ok(None);
        };
        let mut job: Job = from_json(&body)?;
        job.state = JobState::Running;
        job.attempt += 1;
        job.fencing_token += 1;
        job.lease_owner = Some(worker.into());
        job.lease_expires_at = Some(lease_until);
        transaction.execute(
            "UPDATE jobs SET state='running',fencing_token=?1,lease_owner=?2,lease_expires_at=?3,body_json=?4 WHERE tenant_id=?5 AND job_id=?6",
            params![job.fencing_token as i64,worker,lease_until.to_rfc3339(),to_json(&job)?,tenant,job.job_id],
        )?;
        transaction.commit()?;
        Ok(Some(job))
    }

    pub fn update_job_with_fence(&self, job: &Job, expected_fence: u64) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE jobs SET state=?1,lease_owner=?2,lease_expires_at=?3,next_run_at=?4,body_json=?5 WHERE tenant_id=?6 AND job_id=?7 AND fencing_token=?8",
            params![enum_json(&job.state)?,job.lease_owner,job.lease_expires_at.map(|v|v.to_rfc3339()),job.next_run_at.to_rfc3339(),to_json(job)?,job.tenant_id,job.job_id,expected_fence as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict("stale job fencing token".into()));
        }
        Ok(())
    }

    pub fn list_jobs(&self, tenant: &str, limit: usize) -> Result<Vec<Job>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT body_json FROM jobs WHERE tenant_id=?1 ORDER BY next_run_at DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![tenant, limit.min(250) as i64], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| from_json(&row?)).collect()
    }

    fn insert_json<T: Serialize>(
        &self,
        table: &str,
        tenant: &str,
        id_column: &str,
        id: &str,
        extra: Option<(&str, &str)>,
        value: &T,
    ) -> Result<()> {
        let connection = self.connection()?;
        let body = to_json(value)?;
        if let Some((extra_column, extra_value)) = extra {
            let sql = format!(
                "INSERT INTO {table} (tenant_id,{id_column},{extra_column},body_json) VALUES (?1,?2,?3,?4)"
            );
            connection.execute(&sql, params![tenant, id, extra_value, body])?;
        } else {
            let sql =
                format!("INSERT INTO {table} (tenant_id,{id_column},body_json) VALUES (?1,?2,?3)");
            connection.execute(&sql, params![tenant, id, body])?;
        }
        Ok(())
    }

    fn get_json<T: DeserializeOwned, P: rusqlite::Params>(
        &self,
        sql: &str,
        parameters: P,
    ) -> Result<Option<T>> {
        let connection = self.connection()?;
        let body: Option<String> = connection
            .query_row(sql, parameters, |row| row.get(0))
            .optional()?;
        body.map(|value| from_json(&value)).transpose()
    }

    fn list_json<T: DeserializeOwned, P: rusqlite::Params>(
        &self,
        sql: &str,
        parameters: P,
    ) -> Result<Vec<T>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map(parameters, |row| row.get::<_, String>(0))?;
        rows.map(|row| from_json(&row?)).collect()
    }
}

fn insert_memory_tx(transaction: &rusqlite::Transaction<'_>, object: &MemoryObject) -> Result<()> {
    transaction.execute(
        r#"INSERT INTO memory_objects
        (tenant_id,object_id,logical_key,memory_type,source_id,source_version,authority,
         effective_start,effective_end,recorded_at,permissions_json,sensitivity,version,status,
         superseded_by,content_hash,governing,body_json)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)"#,
        params![
            object.tenant_id,
            object.object_id,
            object.logical_key,
            enum_json(&object.memory_type)?,
            object.source_id,
            object.source_version,
            object.authority.rank(),
            object.effective_start.map(|v| v.to_rfc3339()),
            object.effective_end.map(|v| v.to_rfc3339()),
            object.recorded_at.to_rfc3339(),
            to_json(&object.permissions)?,
            object.sensitivity,
            object.version,
            enum_json(&object.status)?,
            object.superseded_by,
            object.content_hash,
            object.governing,
            to_json(object)?
        ],
    )?;
    Ok(())
}

fn insert_audit_tx(connection: &Connection, event: &AuditEvent) -> Result<()> {
    connection.execute(
        "INSERT INTO audit_events (tenant_id,event_id,actor_id,action,target_type,target_id,created_at,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![event.tenant_id,event.event_id,event.actor_id,event.action,event.target_type,event.target_id,event.created_at.to_rfc3339(),to_json(event)?],
    )?;
    Ok(())
}

struct OutboxRow {
    tenant_id: String,
    event_id: String,
    event_type: String,
    aggregate_id: String,
    idempotency_key: String,
    payload: String,
    created_at: String,
    completed_at: Option<String>,
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn from_json<T: DeserializeOwned>(value: &str) -> Result<T> {
    Ok(serde_json::from_str(value)?)
}

fn enum_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?.trim_matches('"').to_string())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AmosError::Validation(format!("invalid timestamp: {error}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::{Authority, MemoryType};

    #[test]
    fn source_version_identity_is_immutable() {
        let store = Store::in_memory().unwrap();
        let first = MemoryObject::new(
            "tenant",
            "metric:x",
            MemoryType::SemanticDefinition,
            "x",
            json!({"v":1}),
            "semantic",
            "v1",
            Authority::OwnerApproved,
        );
        store.write_memory(&first).unwrap();
        store.write_memory(&first).unwrap();
        let mut changed = first.clone();
        changed.object_id = crate::domain::new_id("mem");
        changed.content = json!({"v":2});
        changed.content_hash = crate::domain::content_hash(&changed.content);
        assert!(matches!(
            store.write_memory(&changed),
            Err(AmosError::Conflict(_))
        ));
    }
}
