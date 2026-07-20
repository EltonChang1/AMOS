use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    Result,
    domain::{
        AnalyticalTransaction, Artifact, AtxnState, AuditEvent, Claim, ContextManifest,
        DependencyEdge, ErasureReceipt, ExecutionRecord, Job, JobState, MemoryObject, MemoryStatus,
        OutboxEvent, OutboxState, Outcome, PolicyVisibility, PublicationValidity, ReplayPackage,
        ReplayResult, RetentionRecord, Review, SemanticValidity, TaskDefinition, TypedPlan,
        VerificationRecord, content_hash, stable_id,
    },
    error::AmosError,
};

pub const CURRENT_SCHEMA_VERSION: u32 = 6;

#[derive(Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct InvalidationReceipt {
    tenant_id: String,
    deduplication_key: String,
    request_hash: String,
    target_type: String,
    target_id: String,
    reason: String,
    #[serde(default)]
    after_claim_id: Option<String>,
    affected_claim_ids: Vec<String>,
    next_cursor: Option<String>,
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
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER NOT NULL PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                checksum TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );
            "#,
        )?;
        let installed: Option<u32> = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        if installed.is_some_and(|version| version > CURRENT_SCHEMA_VERSION) {
            return Err(AmosError::Storage(format!(
                "database schema version {} is newer than supported version {CURRENT_SCHEMA_VERSION}",
                installed.unwrap_or_default()
            )));
        }
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;

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
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                tenant_id UNINDEXED,
                object_id UNINDEXED,
                logical_key,
                summary,
                content,
                tokenize = 'unicode61'
            );

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
            CREATE UNIQUE INDEX IF NOT EXISTS idx_execution_effect
                ON executions(tenant_id, atxn_id, step_id, fencing_token);
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
                publication_validity TEXT NOT NULL, replay_availability TEXT NOT NULL,
                semantic_validity TEXT NOT NULL, policy_visibility TEXT NOT NULL,
                review_state TEXT NOT NULL, supersession_state TEXT NOT NULL,
                validity_seq INTEGER NOT NULL DEFAULT 0,
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
            CREATE TABLE IF NOT EXISTS replay_comparisons (
                tenant_id TEXT NOT NULL, replay_atxn_id TEXT NOT NULL,
                artifact_id TEXT NOT NULL, status TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, replay_atxn_id)
            );
            CREATE INDEX IF NOT EXISTS idx_replay_comparison_artifact
                ON replay_comparisons(tenant_id, artifact_id, replay_atxn_id);
            CREATE TABLE IF NOT EXISTS reviews (
                tenant_id TEXT NOT NULL, review_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL, request_hash TEXT NOT NULL,
                reviewer_id TEXT NOT NULL, decision TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, review_id)
            );
            CREATE TABLE IF NOT EXISTS invalidation_receipts (
                tenant_id TEXT NOT NULL, deduplication_key TEXT NOT NULL,
                request_hash TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, deduplication_key)
            );
            CREATE TABLE IF NOT EXISTS audit_events (
                tenant_id TEXT NOT NULL, event_id TEXT NOT NULL, actor_id TEXT NOT NULL,
                action TEXT NOT NULL, target_type TEXT NOT NULL, target_id TEXT NOT NULL,
                created_at TEXT NOT NULL, body_json TEXT NOT NULL,
                PRIMARY KEY (tenant_id, event_id)
            );
            CREATE INDEX IF NOT EXISTS idx_audit_created
                ON audit_events(tenant_id, created_at DESC);
            CREATE TABLE IF NOT EXISTS retention_records (
                tenant_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                retained_until TEXT NOT NULL,
                legal_hold INTEGER NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY(tenant_id,target_type,target_id)
            );
            CREATE INDEX IF NOT EXISTS idx_retention_due
                ON retention_records(tenant_id,legal_hold,retained_until);
            CREATE TABLE IF NOT EXISTS erasure_receipts (
                tenant_id TEXT NOT NULL,
                receipt_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                erased_at TEXT NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY(tenant_id,receipt_id),
                UNIQUE(tenant_id,idempotency_key)
            );
            CREATE TABLE IF NOT EXISTS outbox_events (
                tenant_id TEXT NOT NULL, event_id TEXT NOT NULL, event_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL, idempotency_key TEXT NOT NULL,
                payload_json TEXT NOT NULL, created_at TEXT NOT NULL, completed_at TEXT,
                state TEXT NOT NULL DEFAULT 'ready',
                attempt INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 8,
                fencing_token INTEGER NOT NULL DEFAULT 0,
                lease_owner TEXT, lease_expires_at TEXT, next_attempt_at TEXT, last_error TEXT,
                PRIMARY KEY (tenant_id, event_id), UNIQUE (tenant_id, idempotency_key)
            );
            CREATE INDEX IF NOT EXISTS idx_outbox_pending
                ON outbox_events(tenant_id, completed_at, created_at);
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
        add_column_if_missing(
            &connection,
            "claims",
            "publication_validity",
            "TEXT NOT NULL DEFAULT 'draft'",
        )?;
        add_column_if_missing(
            &connection,
            "claims",
            "replay_availability",
            "TEXT NOT NULL DEFAULT 'level_0'",
        )?;
        add_column_if_missing(
            &connection,
            "claims",
            "validity_seq",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(&connection, "reviews", "idempotency_key", "TEXT")?;
        add_column_if_missing(&connection, "reviews", "request_hash", "TEXT")?;
        add_column_if_missing(
            &connection,
            "outbox_events",
            "state",
            "TEXT NOT NULL DEFAULT 'ready'",
        )?;
        add_column_if_missing(
            &connection,
            "outbox_events",
            "attempt",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            &connection,
            "outbox_events",
            "max_attempts",
            "INTEGER NOT NULL DEFAULT 8",
        )?;
        add_column_if_missing(
            &connection,
            "outbox_events",
            "fencing_token",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(&connection, "outbox_events", "lease_owner", "TEXT")?;
        add_column_if_missing(&connection, "outbox_events", "lease_expires_at", "TEXT")?;
        add_column_if_missing(&connection, "outbox_events", "next_attempt_at", "TEXT")?;
        add_column_if_missing(&connection, "outbox_events", "last_error", "TEXT")?;
        connection.execute_batch(
            r#"
            UPDATE claims
               SET publication_validity =
                       COALESCE(json_extract(body_json, '$.publication_validity'), publication_validity),
                   replay_availability =
                       COALESCE(json_extract(body_json, '$.replay_availability'), replay_availability)
             WHERE json_valid(body_json);
            UPDATE reviews
               SET idempotency_key = COALESCE(idempotency_key, review_id),
                   request_hash = COALESCE(request_hash, review_id);
            UPDATE reviews
               SET body_json = json_set(
                       body_json,
                       '$.idempotency_key', idempotency_key,
                       '$.request_hash', request_hash
                   )
             WHERE json_valid(body_json);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_review_idempotency
                ON reviews(tenant_id, idempotency_key);
            CREATE INDEX IF NOT EXISTS idx_claim_validity
                ON claims(
                    tenant_id, publication_validity, semantic_validity,
                    policy_visibility, replay_availability, review_state,
                    supersession_state
                );
            CREATE INDEX IF NOT EXISTS idx_outbox_delivery
                ON outbox_events(
                    tenant_id, state, next_attempt_at, lease_expires_at, created_at
                );
            INSERT OR IGNORE INTO memory_fts(
                rowid,tenant_id,object_id,logical_key,summary,content
            )
            SELECT rowid,tenant_id,object_id,logical_key,
                   json_extract(body_json,'$.summary'),
                   json_extract(body_json,'$.content')
              FROM memory_objects
             WHERE rowid NOT IN (SELECT rowid FROM memory_fts)
               AND json_valid(body_json);
            "#,
        )?;
        for (version, name, contract) in [
            (1, "control_plane_base", "amos-control-schema-v1"),
            (2, "claim_review_validity", "amos-claim-validity-v2"),
            (3, "memory_fts_backfill", "amos-memory-fts-v3"),
            (4, "outbox_leases", "amos-outbox-leases-v4"),
            (5, "replay_comparisons", "amos-replay-comparison-v5"),
            (6, "retention_erasure", "amos-retention-erasure-v6"),
        ] {
            record_migration(&connection, version, name, contract)?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32> {
        let connection = self.connection()?;
        connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, Option<u32>>(0)
            })?
            .ok_or_else(|| AmosError::Storage("schema migration ledger is empty".into()))
    }

    pub fn write_memory(&self, object: &MemoryObject) -> Result<()> {
        let body = to_json(object)?;
        let permissions = to_json(&object.permissions)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_hash: Option<String> = transaction
            .query_row(
                "SELECT content_hash FROM memory_objects WHERE tenant_id=?1 AND source_id=?2 AND logical_key=?3 AND source_version=?4",
                params![object.tenant_id, object.source_id, object.logical_key, object.source_version],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(hash) = existing_hash {
            transaction.commit()?;
            return if hash == object.content_hash {
                Ok(())
            } else {
                Err(AmosError::Conflict(format!(
                    "source version {} changed content for {}",
                    object.source_version, object.logical_key
                )))
            };
        }
        insert_memory_tx_with_json(&transaction, object, &permissions, &body)?;
        transaction.commit()?;
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

    #[allow(clippy::too_many_arguments)]
    pub fn retrieve_memory_candidates(
        &self,
        tenant_id: &str,
        permissions: &BTreeSet<String>,
        required_types: &BTreeSet<crate::domain::MemoryType>,
        time_start: DateTime<Utc>,
        time_end: DateTime<Utc>,
        fts_query: Option<&str>,
        candidate_limit: usize,
    ) -> Result<Vec<MemoryObject>> {
        if tenant_id.trim().is_empty() || time_start > time_end || candidate_limit == 0 {
            return Err(AmosError::Validation(
                "memory candidate query requires tenant, valid interval, and positive limit".into(),
            ));
        }
        let permissions_json = to_json(permissions)?;
        let types_json = to_json(required_types)?;
        let limit = candidate_limit.min(2_000) as i64;
        let connection = self.connection()?;
        let bodies = if let Some(query) = fts_query.filter(|query| !query.trim().is_empty()) {
            let mut statement = connection.prepare(
                "SELECT m.body_json
                   FROM memory_fts
                   JOIN memory_objects m ON m.rowid=memory_fts.rowid
                  WHERE memory_fts MATCH ?1
                    AND m.tenant_id=?2
                    AND m.status='active'
                    AND m.superseded_by IS NULL
                    AND (m.effective_start IS NULL OR m.effective_start<=?3)
                    AND (m.effective_end IS NULL OR m.effective_end>=?4)
                    AND (
                        json_array_length(?5)=0
                        OR m.memory_type IN (SELECT value FROM json_each(?5))
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM json_each(m.permissions_json) required
                         WHERE required.value NOT IN (
                             SELECT value FROM json_each(?6)
                         )
                    )
                  ORDER BY bm25(memory_fts),m.authority DESC,m.recorded_at DESC,m.object_id
                  LIMIT ?7",
            )?;
            statement
                .query_map(
                    params![
                        query,
                        tenant_id,
                        time_end.to_rfc3339(),
                        time_start.to_rfc3339(),
                        types_json,
                        permissions_json,
                        limit,
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut statement = connection.prepare(
                "SELECT m.body_json
                   FROM memory_objects m
                  WHERE m.tenant_id=?1
                    AND m.status='active'
                    AND m.superseded_by IS NULL
                    AND (m.effective_start IS NULL OR m.effective_start<=?2)
                    AND (m.effective_end IS NULL OR m.effective_end>=?3)
                    AND (
                        json_array_length(?4)=0
                        OR m.memory_type IN (SELECT value FROM json_each(?4))
                    )
                    AND NOT EXISTS (
                        SELECT 1 FROM json_each(m.permissions_json) required
                         WHERE required.value NOT IN (
                             SELECT value FROM json_each(?5)
                         )
                    )
                  ORDER BY m.authority DESC,m.recorded_at DESC,m.object_id
                  LIMIT ?6",
            )?;
            statement
                .query_map(
                    params![
                        tenant_id,
                        time_end.to_rfc3339(),
                        time_start.to_rfc3339(),
                        types_json,
                        permissions_json,
                        limit,
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        bodies.into_iter().map(|body| from_json(&body)).collect()
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

    pub fn get_task_definition_version(
        &self,
        tenant: &str,
        task_type: &str,
        version: u32,
    ) -> Result<Option<TaskDefinition>> {
        self.get_json(
            "SELECT body_json FROM task_definitions
             WHERE tenant_id=?1 AND task_type=?2 AND version=?3",
            params![tenant, task_type, version],
        )
    }

    pub fn create_transaction(
        &self,
        transaction: &AnalyticalTransaction,
    ) -> Result<AnalyticalTransaction> {
        validate_initial_transaction(transaction)?;
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = database
            .query_row(
                "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND idempotency_key=?2",
                params![transaction.tenant_id, transaction.idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: AnalyticalTransaction = from_json(&existing)?;
            database.commit()?;
            return if existing.request_hash == transaction.request_hash {
                Ok(existing)
            } else {
                Err(AmosError::IdempotencyConflict(
                    transaction.idempotency_key.clone(),
                ))
            };
        }
        database.execute(
            "INSERT INTO atxn_transactions (tenant_id,atxn_id,idempotency_key,request_hash,state,state_seq,terminal,updated_at,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![transaction.tenant_id, transaction.atxn_id, transaction.idempotency_key, transaction.request_hash, enum_json(&transaction.state)?, transaction.state_seq as i64, transaction.terminal, transaction.updated_at.to_rfc3339(), to_json(transaction)?],
        )?;
        insert_outbox_tx(
            &database,
            &new_outbox_event(
                &transaction.tenant_id,
                "atxn.admitted",
                &transaction.atxn_id,
                format!("{}/admitted", transaction.atxn_id),
                serde_json::json!({
                    "atxn_id": transaction.atxn_id,
                    "request_id": transaction.request_id,
                    "state": transaction.state,
                    "state_seq": transaction.state_seq,
                }),
            ),
        )?;
        database.commit()?;
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
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let body: Option<String> = database
            .query_row(
                "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND atxn_id=?2",
                params![tenant, atxn_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut transaction: AnalyticalTransaction = body
            .map(|body| from_json(&body))
            .transpose()?
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
        let changed = database.execute(
            "UPDATE atxn_transactions SET state=?1,state_seq=?2,terminal=?3,updated_at=?4,body_json=?5 WHERE tenant_id=?6 AND atxn_id=?7 AND state=?8 AND state_seq=?9 AND terminal=0",
            params![enum_json(&next)?, transaction.state_seq as i64, transaction.terminal, transaction.updated_at.to_rfc3339(), to_json(&transaction)?, tenant, atxn_id, enum_json(&expected)?, expected_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(format!(
                "compare-and-swap failed for {atxn_id}"
            )));
        }
        insert_outbox_tx(
            &database,
            &new_outbox_event(
                tenant,
                "atxn.transitioned",
                atxn_id,
                format!("{atxn_id}/state/{}", transaction.state_seq),
                serde_json::json!({
                    "atxn_id": atxn_id,
                    "from": expected,
                    "to": next,
                    "state_seq": transaction.state_seq,
                    "outcome": transaction.outcome,
                }),
            ),
        )?;
        database.commit()?;
        Ok(transaction)
    }

    pub fn checkpoint_transaction(&self, transaction: &AnalyticalTransaction) -> Result<()> {
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let body: Option<String> = database
            .query_row(
                "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND atxn_id=?2",
                params![transaction.tenant_id, transaction.atxn_id],
                |row| row.get(0),
            )
            .optional()?;
        let current: AnalyticalTransaction = body
            .map(|body| from_json(&body))
            .transpose()?
            .ok_or_else(|| AmosError::NotFound(transaction.atxn_id.clone()))?;
        validate_transaction_checkpoint(&current, transaction)?;
        let changed = database.execute(
            "UPDATE atxn_transactions SET updated_at=?1,body_json=?2 WHERE tenant_id=?3 AND atxn_id=?4 AND state=?5 AND state_seq=?6 AND terminal=0",
            params![transaction.updated_at.to_rfc3339(),to_json(transaction)?,transaction.tenant_id,transaction.atxn_id,enum_json(&transaction.state)?,transaction.state_seq as i64],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "transaction checkpoint lost compare-and-swap".into(),
            ));
        }
        database.commit()?;
        Ok(())
    }

    pub fn save_manifest(&self, manifest: &ContextManifest) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "INSERT OR IGNORE INTO context_manifests
             (tenant_id,manifest_id,atxn_id,body_json) VALUES (?1,?2,?3,?4)",
            params![
                manifest.tenant_id,
                manifest.manifest_id,
                manifest.atxn_id,
                to_json(manifest)?
            ],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let existing: String = connection.query_row(
            "SELECT body_json FROM context_manifests
             WHERE tenant_id=?1 AND manifest_id=?2",
            params![manifest.tenant_id, manifest.manifest_id],
            |row| row.get(0),
        )?;
        if from_json::<ContextManifest>(&existing)? == *manifest {
            Ok(())
        } else {
            Err(AmosError::IdempotencyConflict(manifest.manifest_id.clone()))
        }
    }

    pub fn get_manifest(&self, tenant: &str, id: &str) -> Result<Option<ContextManifest>> {
        self.get_json(
            "SELECT body_json FROM context_manifests WHERE tenant_id=?1 AND manifest_id=?2",
            params![tenant, id],
        )
    }

    pub fn get_manifest_by_atxn(
        &self,
        tenant: &str,
        atxn_id: &str,
    ) -> Result<Option<ContextManifest>> {
        self.get_json(
            "SELECT body_json FROM context_manifests
             WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid DESC LIMIT 1",
            params![tenant, atxn_id],
        )
    }

    pub fn save_plan(&self, plan: &TypedPlan) -> Result<()> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "INSERT OR IGNORE INTO plans
             (tenant_id,plan_id,atxn_id,body_json) VALUES (?1,?2,?3,?4)",
            params![plan.tenant_id, plan.plan_id, plan.atxn_id, to_json(plan)?],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let existing: String = connection.query_row(
            "SELECT body_json FROM plans WHERE tenant_id=?1 AND plan_id=?2",
            params![plan.tenant_id, plan.plan_id],
            |row| row.get(0),
        )?;
        if from_json::<TypedPlan>(&existing)? == *plan {
            Ok(())
        } else {
            Err(AmosError::IdempotencyConflict(plan.plan_id.clone()))
        }
    }

    pub fn get_plan(&self, tenant: &str, id: &str) -> Result<Option<TypedPlan>> {
        self.get_json(
            "SELECT body_json FROM plans WHERE tenant_id=?1 AND plan_id=?2",
            params![tenant, id],
        )
    }

    pub fn get_plan_by_atxn(&self, tenant: &str, atxn_id: &str) -> Result<Option<TypedPlan>> {
        self.get_json(
            "SELECT body_json FROM plans
             WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid DESC LIMIT 1",
            params![tenant, atxn_id],
        )
    }

    pub fn save_execution(&self, execution: &ExecutionRecord) -> Result<ExecutionRecord> {
        if execution.tenant_id.trim().is_empty()
            || execution.execution_id.trim().is_empty()
            || execution.atxn_id.trim().is_empty()
            || execution.step_id.trim().is_empty()
            || execution.output_hash.trim().is_empty()
        {
            return Err(AmosError::Validation(
                "execution persistence requires tenant, execution, A-TXN, step, and output identifiers"
                    .into(),
            ));
        }
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let atxn_body: Option<String> = database
            .query_row(
                "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND atxn_id=?2",
                params![execution.tenant_id, execution.atxn_id],
                |row| row.get(0),
            )
            .optional()?;
        let atxn: AnalyticalTransaction = atxn_body
            .map(|body| from_json(&body))
            .transpose()?
            .ok_or_else(|| AmosError::NotFound(execution.atxn_id.clone()))?;
        if atxn.state != AtxnState::Executing
            || atxn.state_seq != execution.fencing_token
            || atxn.terminal
        {
            return Err(AmosError::Conflict(format!(
                "execution fence is stale for A-TXN {}",
                execution.atxn_id
            )));
        }
        let existing: Option<String> = database
            .query_row(
                "SELECT body_json FROM executions
                 WHERE tenant_id=?1 AND atxn_id=?2 AND step_id=?3 AND fencing_token=?4",
                params![
                    execution.tenant_id,
                    execution.atxn_id,
                    execution.step_id,
                    execution.fencing_token as i64,
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: ExecutionRecord = from_json(&existing)?;
            database.commit()?;
            return if same_execution_effect(&existing, execution) {
                Ok(existing)
            } else {
                Err(AmosError::IdempotencyConflict(format!(
                    "{}/{}/{}",
                    execution.atxn_id, execution.step_id, execution.fencing_token
                )))
            };
        }
        database.execute(
            "INSERT INTO executions (tenant_id,execution_id,atxn_id,step_id,output_hash,fencing_token,body_json) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![execution.tenant_id,execution.execution_id,execution.atxn_id,execution.step_id,execution.output_hash,execution.fencing_token as i64,to_json(execution)?],
        )?;
        insert_outbox_tx(
            &database,
            &new_outbox_event(
                &execution.tenant_id,
                "execution.completed",
                &execution.atxn_id,
                format!(
                    "{}/{}/{}/execution",
                    execution.atxn_id, execution.step_id, execution.fencing_token
                ),
                serde_json::json!({
                    "atxn_id": execution.atxn_id,
                    "execution_id": execution.execution_id,
                    "step_id": execution.step_id,
                    "fencing_token": execution.fencing_token,
                    "output_hash": execution.output_hash,
                }),
            ),
        )?;
        database.commit()?;
        Ok(execution.clone())
    }

    pub fn list_executions(&self, tenant: &str, atxn_id: &str) -> Result<Vec<ExecutionRecord>> {
        self.list_json(
            "SELECT body_json FROM executions WHERE tenant_id=?1 AND atxn_id=?2 ORDER BY rowid",
            params![tenant, atxn_id],
        )
    }

    pub fn save_verification(&self, verification: &VerificationRecord) -> Result<()> {
        let connection = self.connection()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT body_json FROM verification_records
                 WHERE tenant_id=?1 AND verification_id=?2",
                params![verification.tenant_id, verification.verification_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: VerificationRecord = from_json(&existing)?;
            if !same_verification_effect(&existing, verification) {
                return Err(AmosError::IdempotencyConflict(
                    verification.verification_id.clone(),
                ));
            }
            return Ok(());
        }
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
        validate_evidence_bundle(atxn, artifact, claims, edges, package, audit)?;
        let mut committed = atxn.clone();
        committed.state = AtxnState::EvidenceCommitted;
        committed.state_seq += 1;
        committed.updated_at = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_transaction_snapshot(&transaction, atxn)?;
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
                "INSERT INTO claims
                 (tenant_id,claim_id,artifact_id,publication_validity,semantic_validity,
                  policy_visibility,replay_availability,review_state,supersession_state,
                  validity_seq,body_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10)",
                params![
                    claim.tenant_id,
                    claim.claim_id,
                    claim.artifact_id,
                    enum_json(&claim.publication_validity)?,
                    enum_json(&claim.semantic_validity)?,
                    enum_json(&claim.policy_visibility)?,
                    enum_json(&claim.replay_availability)?,
                    enum_json(&claim.review_state)?,
                    enum_json(&claim.supersession_state)?,
                    to_json(claim)?
                ],
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
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                &artifact.tenant_id,
                "evidence.committed",
                &artifact.artifact_id,
                format!("{}/evidence", artifact.artifact_id),
                serde_json::json!({
                    "artifact_id": artifact.artifact_id,
                    "atxn_id": atxn.atxn_id,
                    "state_seq": committed.state_seq,
                }),
            ),
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

    pub fn list_artifacts_after(
        &self,
        tenant: &str,
        after_artifact_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Artifact>> {
        if tenant.trim().is_empty() || limit == 0 {
            return Err(AmosError::Validation(
                "artifact page requires tenant and positive limit".into(),
            ));
        }
        let connection = self.connection()?;
        let after_rowid = if let Some(artifact_id) = after_artifact_id {
            connection
                .query_row(
                    "SELECT rowid FROM artifacts WHERE tenant_id=?1 AND artifact_id=?2",
                    params![tenant, artifact_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|_| AmosError::Validation("artifact cursor is invalid".into()))?
        } else {
            i64::MAX
        };
        let mut statement = connection.prepare(
            "SELECT body_json FROM artifacts
             WHERE tenant_id=?1 AND rowid<?2 ORDER BY rowid DESC LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![tenant, after_rowid, limit.min(100) as i64], |row| {
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
        validate_publication_bundle(atxn, artifact, claims, audit)?;
        let mut published = atxn.clone();
        published.state = AtxnState::Published;
        published.state_seq += 1;
        published.updated_at = Utc::now();
        published.outcome = Some(outcome);
        let mut published_artifact = artifact.clone();
        published_artifact.publication_validity = PublicationValidity::ValidAtPublication;
        let mut published_claims = claims.to_vec();
        for claim in &mut published_claims {
            claim.publication_validity = PublicationValidity::ValidAtPublication;
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_transaction_snapshot(&transaction, atxn)?;
        ensure_artifact_snapshot(&transaction, artifact)?;
        for claim in claims.iter() {
            ensure_claim_snapshot(&transaction, claim)?;
        }
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
            params![to_json(&published_artifact)?,artifact.tenant_id,artifact.artifact_id],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "artifact publication validity changed concurrently".into(),
            ));
        }
        for claim in &published_claims {
            let changed = transaction.execute(
                "UPDATE claims
                    SET publication_validity=?1, validity_seq=validity_seq+1, body_json=?2
                  WHERE tenant_id=?3 AND artifact_id=?4 AND claim_id=?5
                    AND publication_validity='draft'",
                params![
                    enum_json(&claim.publication_validity)?,
                    to_json(claim)?,
                    claim.tenant_id,
                    claim.artifact_id,
                    claim.claim_id
                ],
            )?;
            if changed != 1 {
                return Err(AmosError::Conflict(format!(
                    "claim publication validity changed concurrently for {}",
                    claim.claim_id
                )));
            }
        }
        insert_audit_tx(&transaction, audit)?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                &artifact.tenant_id,
                "artifact.published",
                &artifact.artifact_id,
                format!("{}/published", artifact.artifact_id),
                serde_json::json!({
                    "artifact_id": artifact.artifact_id,
                    "atxn_id": atxn.atxn_id,
                    "state_seq": published.state_seq,
                }),
            ),
        )?;
        transaction.commit()?;
        *artifact = published_artifact;
        claims.clone_from_slice(&published_claims);
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

    pub fn invalidate_claims_page(
        &self,
        tenant: &str,
        target_type: &str,
        target_id: &str,
        reason: &str,
        deduplication_key: &str,
        page_size: usize,
    ) -> Result<Vec<String>> {
        self.invalidate_claims_page_after(
            tenant,
            target_type,
            target_id,
            reason,
            deduplication_key,
            deduplication_key,
            None,
            page_size,
            10_000,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn invalidate_claims_page_after(
        &self,
        tenant: &str,
        target_type: &str,
        target_id: &str,
        reason: &str,
        page_deduplication_key: &str,
        root_invalidation_key: &str,
        after_claim_id: Option<&str>,
        page_size: usize,
        traversal_node_quota: usize,
    ) -> Result<Vec<String>> {
        if tenant.trim().is_empty()
            || target_type.trim().is_empty()
            || target_id.trim().is_empty()
            || reason.trim().is_empty()
            || page_deduplication_key.trim().is_empty()
            || root_invalidation_key.trim().is_empty()
            || page_size == 0
            || traversal_node_quota == 0
        {
            return Err(AmosError::Validation(
                "invalidation requires tenant, target, reason, page/root keys, page size, and traversal quota".into(),
            ));
        }
        let page_size = page_size.min(250);
        let request_hash = crate::domain::content_hash(&serde_json::json!({
            "tenant_id": tenant,
            "target_type": target_type,
            "target_id": target_id,
            "reason": reason,
            "after_claim_id": after_claim_id,
            "traversal_node_quota": traversal_node_quota,
        }))?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT body_json FROM invalidation_receipts
                  WHERE tenant_id=?1 AND deduplication_key=?2",
                params![tenant, page_deduplication_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: InvalidationReceipt = from_json(&existing)?;
            transaction.commit()?;
            return if existing.request_hash == request_hash {
                Ok(existing.affected_claim_ids)
            } else {
                Err(AmosError::IdempotencyConflict(
                    page_deduplication_key.into(),
                ))
            };
        }
        let reachable = reachable_claim_ids(
            &transaction,
            tenant,
            target_type,
            target_id,
            traversal_node_quota,
        )?;
        let page_ids = reachable
            .into_iter()
            .filter(|claim_id| after_claim_id.is_none_or(|cursor| claim_id.as_str() > cursor))
            .take(page_size + 1)
            .collect::<Vec<_>>();
        let candidates = page_ids
            .iter()
            .map(|claim_id| {
                let body = transaction.query_row(
                    "SELECT body_json FROM claims WHERE tenant_id=?1 AND claim_id=?2",
                    params![tenant, claim_id],
                    |row| row.get::<_, String>(0),
                )?;
                Ok((claim_id.clone(), body))
            })
            .collect::<Result<Vec<_>>>()?;
        let has_more = candidates.len() > page_size;
        let candidates = candidates.into_iter().take(page_size).collect::<Vec<_>>();
        let next_cursor = has_more
            .then(|| candidates.last().map(|(claim_id, _)| claim_id.clone()))
            .flatten();
        let mut affected_claim_ids = Vec::new();
        for (_, body) in candidates {
            let expected: Claim = from_json(&body)?;
            if expected.semantic_validity != crate::domain::SemanticValidity::Current {
                continue;
            }
            let mut updated = expected.clone();
            updated.semantic_validity = crate::domain::SemanticValidity::PendingRevalidation;
            let changed = update_claim_validity_tx(&transaction, &updated)?;
            if changed != 1 {
                return Err(AmosError::Conflict(format!(
                    "claim changed concurrently during invalidation: {}",
                    updated.claim_id
                )));
            }
            let job = Job::ready(
                tenant,
                "claim.revalidate",
                serde_json::json!({
                    "claim_id": updated.claim_id,
                    "artifact_id": updated.artifact_id,
                    "reason": reason,
                    "invalidation_key": root_invalidation_key,
                }),
                format!(
                    "invalidation/{root_invalidation_key}/claim/{}",
                    updated.claim_id
                ),
                5,
            );
            enqueue_job_tx(&transaction, &job)?;
            insert_outbox_tx(
                &transaction,
                &new_outbox_event(
                    tenant,
                    "claim.validity_changed",
                    &updated.claim_id,
                    format!(
                        "invalidation/{root_invalidation_key}/claim/{}/validity",
                        updated.claim_id
                    ),
                    serde_json::json!({
                        "claim_id": updated.claim_id,
                        "artifact_id": updated.artifact_id,
                        "cause": reason,
                        "before": claim_validity_json(&expected),
                        "after": claim_validity_json(&updated),
                    }),
                ),
            )?;
            affected_claim_ids.push(updated.claim_id);
        }
        if let Some(cursor) = &next_cursor {
            let continuation = Job::ready(
                tenant,
                "invalidation.continue",
                serde_json::json!({
                    "target_type": target_type,
                    "target_id": target_id,
                    "reason": reason,
                    "invalidation_key": root_invalidation_key,
                    "after_claim_id": cursor,
                    "page_size": page_size,
                    "traversal_node_quota": traversal_node_quota,
                }),
                format!("invalidation/{root_invalidation_key}/continue/{cursor}"),
                5,
            );
            enqueue_job_tx(&transaction, &continuation)?;
        }
        let receipt = InvalidationReceipt {
            tenant_id: tenant.into(),
            deduplication_key: page_deduplication_key.into(),
            request_hash,
            target_type: target_type.into(),
            target_id: target_id.into(),
            reason: reason.into(),
            after_claim_id: after_claim_id.map(str::to_string),
            affected_claim_ids: affected_claim_ids.clone(),
            next_cursor,
        };
        transaction.execute(
            "INSERT INTO invalidation_receipts
             (tenant_id,deduplication_key,request_hash,body_json)
             VALUES (?1,?2,?3,?4)",
            params![
                receipt.tenant_id,
                receipt.deduplication_key,
                receipt.request_hash,
                to_json(&receipt)?
            ],
        )?;
        insert_audit_tx(
            &transaction,
            &AuditEvent {
                event_id: crate::domain::new_id("audit"),
                tenant_id: tenant.into(),
                actor_id: "system:invalidator".into(),
                action: "claim.invalidate".into(),
                target_type: target_type.into(),
                target_id: target_id.into(),
                request_id: None,
                atxn_id: None,
                outcome: "pass".into(),
                policy_epoch: 0,
                details: serde_json::json!({
                    "reason": reason,
                    "deduplication_key": page_deduplication_key,
                    "root_invalidation_key": root_invalidation_key,
                    "after_claim_id": after_claim_id,
                    "affected_claims": affected_claim_ids.len(),
                    "continuation_required": receipt.next_cursor.is_some(),
                }),
                created_at: Utc::now(),
            },
        )?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                tenant,
                "invalidation.processed",
                target_id,
                format!("invalidation/{page_deduplication_key}/processed"),
                serde_json::json!({
                    "target_type": target_type,
                    "target_id": target_id,
                    "reason": reason,
                    "affected_claim_ids": affected_claim_ids,
                    "next_cursor": receipt.next_cursor,
                }),
            ),
        )?;
        transaction.commit()?;
        Ok(receipt.affected_claim_ids)
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

    pub fn get_replay_result(
        &self,
        tenant: &str,
        replay_atxn_id: &str,
    ) -> Result<Option<ReplayResult>> {
        self.get_json(
            "SELECT body_json FROM replay_comparisons
             WHERE tenant_id=?1 AND replay_atxn_id=?2",
            params![tenant, replay_atxn_id],
        )
    }

    pub fn commit_replay_result(
        &self,
        atxn: &AnalyticalTransaction,
        result: &ReplayResult,
        audit: &AuditEvent,
    ) -> Result<AnalyticalTransaction> {
        if atxn.state != AtxnState::Revalidating
            || result.replay_atxn_id != atxn.atxn_id
            || result.artifact_id.trim().is_empty()
            || result.original_atxn_id.trim().is_empty()
            || result.comparisons.is_empty()
            || audit.tenant_id != atxn.tenant_id
            || audit.atxn_id.as_deref() != Some(atxn.atxn_id.as_str())
            || audit.action != "artifact.replay.compare"
            || audit.target_id != result.artifact_id
        {
            return Err(AmosError::Validation(
                "replay comparison, transaction, and audit are inconsistent".into(),
            ));
        }
        let comparison_ids = result
            .comparisons
            .iter()
            .map(|comparison| comparison.replay_execution_id.as_str())
            .collect::<BTreeSet<_>>();
        if comparison_ids.len() != result.comparisons.len()
            || result
                .matching_execution_ids
                .iter()
                .chain(&result.changed_execution_ids)
                .any(|id| !comparison_ids.contains(id.as_str()))
        {
            return Err(AmosError::Validation(
                "replay execution comparison identifiers are inconsistent".into(),
            ));
        }
        let mut committed = atxn.clone();
        committed.state = AtxnState::EvidenceCommitted;
        committed.state_seq += 1;
        committed.outcome = Some(result.status);
        committed.updated_at = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_transaction_snapshot(&transaction, atxn)?;
        for execution_id in comparison_ids {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM executions
                     WHERE tenant_id=?1 AND atxn_id=?2 AND execution_id=?3
                 )",
                params![atxn.tenant_id, atxn.atxn_id, execution_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(AmosError::Validation(format!(
                    "replay execution {execution_id} is not durable"
                )));
            }
        }
        let changed = transaction.execute(
            "UPDATE atxn_transactions
                SET state='evidence_committed',state_seq=?1,updated_at=?2,body_json=?3
              WHERE tenant_id=?4 AND atxn_id=?5
                AND state='revalidating' AND state_seq=?6 AND terminal=0",
            params![
                committed.state_seq as i64,
                committed.updated_at.to_rfc3339(),
                to_json(&committed)?,
                atxn.tenant_id,
                atxn.atxn_id,
                atxn.state_seq as i64
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "replay comparison lost transaction compare-and-swap".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO replay_comparisons
             (tenant_id,replay_atxn_id,artifact_id,status,body_json)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                atxn.tenant_id,
                atxn.atxn_id,
                result.artifact_id,
                enum_json(&result.status)?,
                to_json(result)?
            ],
        )?;
        insert_audit_tx(&transaction, audit)?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                &atxn.tenant_id,
                "artifact.replay.compared",
                &result.artifact_id,
                format!("{}/replay/comparison", atxn.atxn_id),
                serde_json::json!({
                    "artifact_id": result.artifact_id,
                    "original_atxn_id": result.original_atxn_id,
                    "replay_atxn_id": result.replay_atxn_id,
                    "status": result.status,
                    "matching_execution_ids": result.matching_execution_ids,
                    "changed_execution_ids": result.changed_execution_ids,
                }),
            ),
        )?;
        transaction.commit()?;
        Ok(committed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn commit_review(
        &self,
        review: &Review,
        artifact: &Artifact,
        expected_claims: &[Claim],
        updated_claims: &[Claim],
        feedback: &MemoryObject,
        audit: &AuditEvent,
        revalidation_job: &Job,
    ) -> Result<Review> {
        validate_review_bundle(
            review,
            artifact,
            expected_claims,
            updated_claims,
            feedback,
            audit,
            revalidation_job,
        )?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT body_json FROM reviews
                  WHERE tenant_id=?1 AND idempotency_key=?2",
                params![review.tenant_id, review.idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: Review = from_json(&existing)?;
            transaction.commit()?;
            return if existing.request_hash == review.request_hash {
                Ok(existing)
            } else {
                Err(AmosError::IdempotencyConflict(
                    review.idempotency_key.clone(),
                ))
            };
        }

        ensure_artifact_snapshot(&transaction, artifact)?;
        let expected_by_id = expected_claims
            .iter()
            .map(|claim| (claim.claim_id.as_str(), claim))
            .collect::<BTreeMap<_, _>>();
        for expected in expected_claims {
            ensure_claim_snapshot(&transaction, expected)?;
        }
        transaction.execute(
            "INSERT INTO reviews
             (tenant_id,review_id,artifact_id,idempotency_key,request_hash,reviewer_id,
              decision,body_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                review.tenant_id,
                review.review_id,
                review.artifact_id,
                review.idempotency_key,
                review.request_hash,
                review.reviewer_id,
                enum_json(&review.decision)?,
                to_json(review)?
            ],
        )?;
        for updated in updated_claims {
            let expected = expected_by_id
                .get(updated.claim_id.as_str())
                .ok_or_else(|| AmosError::Validation("review claim set changed".into()))?;
            if *expected == updated {
                continue;
            }
            let changed = update_claim_validity_tx(&transaction, updated)?;
            if changed != 1 {
                return Err(AmosError::Conflict(format!(
                    "claim changed concurrently during review: {}",
                    updated.claim_id
                )));
            }
            insert_outbox_tx(
                &transaction,
                &new_outbox_event(
                    &review.tenant_id,
                    "claim.validity_changed",
                    &updated.claim_id,
                    format!("{}/claim/{}", review.review_id, updated.claim_id),
                    serde_json::json!({
                        "claim_id": updated.claim_id,
                        "artifact_id": updated.artifact_id,
                        "cause": "human_review",
                        "before": claim_validity_json(expected),
                        "after": claim_validity_json(updated),
                        "review_id": review.review_id,
                    }),
                ),
            )?;
        }
        insert_memory_tx(&transaction, feedback)?;
        insert_audit_tx(&transaction, audit)?;
        enqueue_job_tx(&transaction, revalidation_job)?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                &review.tenant_id,
                "review.appended",
                &review.review_id,
                format!("{}/appended", review.review_id),
                serde_json::json!({
                    "review_id": review.review_id,
                    "artifact_id": review.artifact_id,
                    "claim_ids": review.claim_ids,
                    "decision": review.decision,
                    "authority": review.authority,
                }),
            ),
        )?;
        transaction.commit()?;
        Ok(review.clone())
    }

    pub fn get_review_by_idempotency_key(
        &self,
        tenant: &str,
        idempotency_key: &str,
    ) -> Result<Option<Review>> {
        self.get_json(
            "SELECT body_json FROM reviews WHERE tenant_id=?1 AND idempotency_key=?2",
            params![tenant, idempotency_key],
        )
    }

    pub fn commit_claim_validity_updates(
        &self,
        expected_claims: &[Claim],
        updated_claims: &[Claim],
        audit: &AuditEvent,
        cause: &str,
    ) -> Result<Vec<Claim>> {
        validate_claim_validity_batch(expected_claims, updated_claims, audit, cause)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let expected_by_id = expected_claims
            .iter()
            .map(|claim| (claim.claim_id.as_str(), claim))
            .collect::<BTreeMap<_, _>>();
        for expected in expected_claims {
            ensure_claim_snapshot(&transaction, expected)?;
        }
        for updated in updated_claims {
            let expected = expected_by_id
                .get(updated.claim_id.as_str())
                .ok_or_else(|| AmosError::Validation("claim validity set changed".into()))?;
            if *expected == updated {
                continue;
            }
            let changed = update_claim_validity_tx(&transaction, updated)?;
            if changed != 1 {
                return Err(AmosError::Conflict(format!(
                    "claim changed concurrently during validity update: {}",
                    updated.claim_id
                )));
            }
            insert_outbox_tx(
                &transaction,
                &new_outbox_event(
                    &updated.tenant_id,
                    "claim.validity_changed",
                    &updated.claim_id,
                    format!("{}/claim/{}", audit.event_id, updated.claim_id),
                    serde_json::json!({
                        "claim_id": updated.claim_id,
                        "artifact_id": updated.artifact_id,
                        "cause": cause,
                        "before": claim_validity_json(expected),
                        "after": claim_validity_json(updated),
                    }),
                ),
            )?;
        }
        insert_audit_tx(&transaction, audit)?;
        transaction.commit()?;
        Ok(updated_claims.to_vec())
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

    pub fn set_retention(
        &self,
        record: &RetentionRecord,
        idempotency_key: &str,
    ) -> Result<RetentionRecord> {
        if record.tenant_id.trim().is_empty()
            || record.target_type.trim().is_empty()
            || record.target_id.trim().is_empty()
            || record.reason.trim().is_empty()
            || record.updated_by.trim().is_empty()
            || idempotency_key.trim().is_empty()
        {
            return Err(AmosError::Validation(
                "retention mutation requires scoped target, reason, actor, and idempotency key"
                    .into(),
            ));
        }
        let request_hash = content_hash(&serde_json::json!({
            "tenant_id": record.tenant_id,
            "target_type": record.target_type,
            "target_id": record.target_id,
            "retained_until": record.retained_until,
            "legal_hold": record.legal_hold,
            "reason": record.reason,
            "updated_by": record.updated_by,
        }))?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_command: Option<String> = transaction
            .query_row(
                "SELECT payload_json FROM outbox_events
                 WHERE tenant_id=?1 AND idempotency_key=?2",
                params![record.tenant_id, idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(payload) = existing_command {
            let payload: serde_json::Value = from_json(&payload)?;
            if payload
                .get("request_hash")
                .and_then(serde_json::Value::as_str)
                != Some(request_hash.as_str())
            {
                return Err(AmosError::IdempotencyConflict(idempotency_key.into()));
            }
            let body: String = transaction.query_row(
                "SELECT body_json FROM retention_records
                 WHERE tenant_id=?1 AND target_type=?2 AND target_id=?3",
                params![record.tenant_id, record.target_type, record.target_id],
                |row| row.get(0),
            )?;
            transaction.commit()?;
            return from_json(&body);
        }
        transaction.execute(
            "INSERT INTO retention_records
             (tenant_id,target_type,target_id,retained_until,legal_hold,body_json)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(tenant_id,target_type,target_id) DO UPDATE SET
                retained_until=excluded.retained_until,
                legal_hold=excluded.legal_hold,
                body_json=excluded.body_json",
            params![
                record.tenant_id,
                record.target_type,
                record.target_id,
                record.retained_until.to_rfc3339(),
                record.legal_hold,
                to_json(record)?,
            ],
        )?;
        insert_audit_tx(
            &transaction,
            &AuditEvent {
                event_id: stable_id(
                    "audit",
                    &(&record.tenant_id, idempotency_key, "retention.set"),
                )?,
                tenant_id: record.tenant_id.clone(),
                actor_id: record.updated_by.clone(),
                action: if record.legal_hold {
                    "retention.legal_hold".into()
                } else {
                    "retention.set".into()
                },
                target_type: record.target_type.clone(),
                target_id: record.target_id.clone(),
                request_id: None,
                atxn_id: None,
                outcome: "pass".into(),
                policy_epoch: 0,
                details: serde_json::json!({
                    "retained_until": record.retained_until,
                    "legal_hold": record.legal_hold,
                    "reason": record.reason,
                }),
                created_at: record.updated_at,
            },
        )?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                &record.tenant_id,
                "retention.changed",
                &record.target_id,
                idempotency_key.into(),
                serde_json::json!({
                    "target_type": record.target_type,
                    "target_id": record.target_id,
                    "request_hash": request_hash,
                    "legal_hold": record.legal_hold,
                    "retained_until": record.retained_until,
                }),
            ),
        )?;
        transaction.commit()?;
        Ok(record.clone())
    }

    pub fn get_retention(
        &self,
        tenant: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<Option<RetentionRecord>> {
        self.get_json(
            "SELECT body_json FROM retention_records
             WHERE tenant_id=?1 AND target_type=?2 AND target_id=?3",
            params![tenant, target_type, target_id],
        )
    }

    pub fn erase_memory(
        &self,
        tenant: &str,
        object_id: &str,
        requested_by: &str,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> Result<ErasureReceipt> {
        if tenant.trim().is_empty()
            || object_id.trim().is_empty()
            || requested_by.trim().is_empty()
            || idempotency_key.trim().is_empty()
        {
            return Err(AmosError::Validation(
                "erasure requires tenant, memory object, actor, and idempotency key".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT body_json FROM erasure_receipts
                 WHERE tenant_id=?1 AND idempotency_key=?2",
                params![tenant, idempotency_key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let receipt: ErasureReceipt = from_json(&existing)?;
            transaction.commit()?;
            return if receipt.target_type == "memory" && receipt.target_id == object_id {
                Ok(receipt)
            } else {
                Err(AmosError::IdempotencyConflict(idempotency_key.into()))
            };
        }
        let retention_body: String = transaction
            .query_row(
                "SELECT body_json FROM retention_records
                 WHERE tenant_id=?1 AND target_type='memory' AND target_id=?2",
                params![tenant, object_id],
                |row| row.get(0),
            )
            .map_err(|_| AmosError::NotFound("memory retention record".into()))?;
        let retention: RetentionRecord = from_json(&retention_body)?;
        if retention.legal_hold {
            return Err(AmosError::Conflict(
                "memory object is protected by a legal hold".into(),
            ));
        }
        if retention.retained_until > now {
            return Err(AmosError::Conflict(
                "memory object has not reached its retention deadline".into(),
            ));
        }
        let (rowid, body): (i64, String) = transaction
            .query_row(
                "SELECT rowid,body_json FROM memory_objects
                 WHERE tenant_id=?1 AND object_id=?2",
                params![tenant, object_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| AmosError::NotFound(object_id.into()))?;
        let object: MemoryObject = from_json(&body)?;
        let affected_claim_ids =
            reachable_claim_ids(&transaction, tenant, "memory", object_id, 10_000)?;
        for claim_id in &affected_claim_ids {
            let body: String = transaction.query_row(
                "SELECT body_json FROM claims WHERE tenant_id=?1 AND claim_id=?2",
                params![tenant, claim_id],
                |row| row.get(0),
            )?;
            let mut claim: Claim = from_json(&body)?;
            claim.semantic_validity = SemanticValidity::Invalid;
            claim.policy_visibility = PolicyVisibility::Redacted;
            update_claim_validity_tx(&transaction, &claim)?;
        }
        transaction.execute("DELETE FROM memory_fts WHERE rowid=?1", [rowid])?;
        let deleted = transaction.execute(
            "DELETE FROM memory_objects WHERE tenant_id=?1 AND object_id=?2",
            params![tenant, object_id],
        )?;
        if deleted != 1 {
            return Err(AmosError::Conflict(
                "memory erasure lost its object snapshot".into(),
            ));
        }
        let affected_claim_ids = affected_claim_ids.into_iter().collect::<Vec<_>>();
        let receipt = ErasureReceipt {
            receipt_id: stable_id("erase", &(tenant, object_id, idempotency_key))?,
            tenant_id: tenant.into(),
            target_type: "memory".into(),
            target_id: object_id.into(),
            erased_content_hash: object.content_hash,
            affected_claim_ids,
            idempotency_key: idempotency_key.into(),
            requested_by: requested_by.into(),
            erased_at: now,
        };
        transaction.execute(
            "INSERT INTO erasure_receipts
             (tenant_id,receipt_id,idempotency_key,target_type,target_id,erased_at,body_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                receipt.tenant_id,
                receipt.receipt_id,
                receipt.idempotency_key,
                receipt.target_type,
                receipt.target_id,
                receipt.erased_at.to_rfc3339(),
                to_json(&receipt)?,
            ],
        )?;
        insert_audit_tx(
            &transaction,
            &AuditEvent {
                event_id: stable_id("audit", &(tenant, idempotency_key, "memory.erase"))?,
                tenant_id: tenant.into(),
                actor_id: requested_by.into(),
                action: "memory.erase".into(),
                target_type: "memory".into(),
                target_id: object_id.into(),
                request_id: None,
                atxn_id: None,
                outcome: "pass".into(),
                policy_epoch: 0,
                details: serde_json::json!({
                    "receipt_id": receipt.receipt_id,
                    "erased_content_hash": receipt.erased_content_hash,
                    "affected_claim_count": receipt.affected_claim_ids.len(),
                }),
                created_at: now,
            },
        )?;
        insert_outbox_tx(
            &transaction,
            &new_outbox_event(
                tenant,
                "memory.erased",
                object_id,
                idempotency_key.into(),
                serde_json::json!({
                    "receipt_id": receipt.receipt_id,
                    "target_id": object_id,
                    "affected_claim_ids": receipt.affected_claim_ids,
                }),
            ),
        )?;
        transaction.commit()?;
        Ok(receipt)
    }

    pub fn has_audit_event(&self, tenant: &str, event_id: &str) -> Result<bool> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM audit_events WHERE tenant_id=?1 AND event_id=?2
                 )",
                params![tenant, event_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn list_audit(&self, tenant: &str, limit: usize) -> Result<Vec<AuditEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare("SELECT body_json FROM audit_events WHERE tenant_id=?1 ORDER BY created_at DESC LIMIT ?2")?;
        let rows = statement.query_map(params![tenant, limit.min(250) as i64], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| from_json(&row?)).collect()
    }

    pub fn list_outbox(&self, tenant: &str, limit: usize) -> Result<Vec<OutboxEvent>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,
                    created_at,completed_at,state,attempt,max_attempts,fencing_token,
                    lease_owner,lease_expires_at,next_attempt_at,last_error
             FROM outbox_events WHERE tenant_id=?1 ORDER BY created_at,event_id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![tenant, limit.min(500) as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, u32>(9)?,
                    row.get::<_, u32>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(
                    tenant_id,
                    event_id,
                    event_type,
                    aggregate_id,
                    idempotency_key,
                    payload_json,
                    created_at,
                    completed_at,
                    state,
                    attempt,
                    max_attempts,
                    fencing_token,
                    lease_owner,
                    lease_expires_at,
                    next_attempt_at,
                    last_error,
                )| {
                    Ok(OutboxEvent {
                        tenant_id,
                        event_id,
                        event_type,
                        aggregate_id,
                        idempotency_key,
                        payload: from_json(&payload_json)?,
                        created_at: parse_timestamp(&created_at)?,
                        completed_at: completed_at.as_deref().map(parse_timestamp).transpose()?,
                        state: parse_outbox_state(&state)?,
                        attempt,
                        max_attempts,
                        fencing_token: u64::try_from(fencing_token).map_err(|_| {
                            AmosError::Storage("negative outbox fencing token".into())
                        })?,
                        lease_owner,
                        lease_expires_at: lease_expires_at
                            .as_deref()
                            .map(parse_timestamp)
                            .transpose()?,
                        next_attempt_at: next_attempt_at
                            .as_deref()
                            .map(parse_timestamp)
                            .transpose()?,
                        last_error,
                    })
                },
            )
            .collect()
    }

    pub fn acquire_outbox(
        &self,
        tenant: &str,
        dispatcher: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<OutboxEvent>> {
        if tenant.trim().is_empty() || dispatcher.trim().is_empty() || lease_until <= now {
            return Err(AmosError::Validation(
                "outbox acquisition requires tenant, dispatcher, and a future lease".into(),
            ));
        }
        let ready = enum_json(&OutboxState::Ready)?;
        let retry_wait = enum_json(&OutboxState::RetryWait)?;
        let running = enum_json(&OutboxState::Running)?;
        let dead_letter = enum_json(&OutboxState::DeadLetter)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        loop {
            let event_id: Option<String> = transaction
                .query_row(
                    "SELECT event_id FROM outbox_events
                      WHERE tenant_id=?1 AND completed_at IS NULL AND (
                         (state IN (?2,?3)
                          AND (next_attempt_at IS NULL OR next_attempt_at<=?4))
                         OR (state=?5 AND lease_expires_at IS NOT NULL AND lease_expires_at<=?4)
                      )
                      ORDER BY CASE state WHEN ?5 THEN 0 ELSE 1 END, created_at,event_id
                      LIMIT 1",
                    params![tenant, ready, retry_wait, now.to_rfc3339(), running],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(event_id) = event_id else {
                transaction.commit()?;
                return Ok(None);
            };
            let mut event = get_outbox_event_tx(&transaction, tenant, &event_id)?
                .ok_or_else(|| AmosError::NotFound(event_id.clone()))?;
            let previous_state = event.state;
            let previous_fence = event.fencing_token;
            if previous_state == OutboxState::Running && event.attempt >= event.max_attempts {
                event.state = OutboxState::DeadLetter;
                event.completed_at = Some(now);
                event.lease_owner = None;
                event.lease_expires_at = None;
                event.last_error = Some("delivery lease expired after final attempt".into());
                let changed = transaction.execute(
                    "UPDATE outbox_events
                        SET state=?1,completed_at=?2,lease_owner=NULL,lease_expires_at=NULL,
                            last_error=?3
                      WHERE tenant_id=?4 AND event_id=?5 AND state=?6 AND fencing_token=?7
                        AND lease_expires_at IS NOT NULL AND lease_expires_at<=?2",
                    params![
                        dead_letter,
                        now.to_rfc3339(),
                        event.last_error,
                        tenant,
                        event.event_id,
                        running,
                        previous_fence as i64
                    ],
                )?;
                if changed != 1 {
                    return Err(AmosError::Conflict(
                        "outbox lease changed while dead-lettering".into(),
                    ));
                }
                insert_audit_tx(
                    &transaction,
                    &AuditEvent {
                        event_id: crate::domain::new_id("audit"),
                        tenant_id: tenant.into(),
                        actor_id: "system:outbox-dispatcher".into(),
                        action: "outbox.dead_letter".into(),
                        target_type: "outbox_event".into(),
                        target_id: event.event_id,
                        request_id: None,
                        atxn_id: None,
                        outcome: "reject".into(),
                        policy_epoch: 0,
                        details: serde_json::json!({"reason":event.last_error}),
                        created_at: now,
                    },
                )?;
                continue;
            }
            event.state = OutboxState::Running;
            event.attempt = event.attempt.saturating_add(1);
            event.fencing_token = event.fencing_token.saturating_add(1);
            event.lease_owner = Some(dispatcher.into());
            event.lease_expires_at = Some(lease_until);
            event.next_attempt_at = None;
            event.last_error = None;
            let changed = transaction.execute(
                "UPDATE outbox_events
                    SET state=?1,attempt=?2,fencing_token=?3,lease_owner=?4,
                        lease_expires_at=?5,next_attempt_at=NULL,last_error=NULL
                  WHERE tenant_id=?6 AND event_id=?7 AND state=?8 AND fencing_token=?9",
                params![
                    enum_json(&event.state)?,
                    event.attempt,
                    event.fencing_token as i64,
                    dispatcher,
                    lease_until.to_rfc3339(),
                    tenant,
                    event.event_id,
                    enum_json(&previous_state)?,
                    previous_fence as i64
                ],
            )?;
            if changed != 1 {
                return Err(AmosError::Conflict(
                    "outbox acquisition lost compare-and-swap".into(),
                ));
            }
            transaction.commit()?;
            return Ok(Some(event));
        }
    }

    pub fn finish_outbox(
        &self,
        event: &OutboxEvent,
        expected_fence: u64,
        expected_owner: &str,
        now: DateTime<Utc>,
        delivery_error: Option<String>,
    ) -> Result<OutboxEvent> {
        if delivery_error
            .as_ref()
            .is_some_and(|error| error.trim().is_empty())
        {
            return Err(AmosError::Validation(
                "outbox delivery error must be non-empty".into(),
            ));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = get_outbox_event_tx(&transaction, &event.tenant_id, &event.event_id)?
            .ok_or_else(|| AmosError::NotFound(event.event_id.clone()))?;
        if current != *event
            || current.state != OutboxState::Running
            || current.fencing_token != expected_fence
            || current.lease_owner.as_deref() != Some(expected_owner)
            || current.lease_expires_at.is_none_or(|expiry| expiry <= now)
        {
            return Err(AmosError::Conflict(
                "outbox delivery lease is stale or expired".into(),
            ));
        }
        let mut updated = current;
        updated.lease_owner = None;
        updated.lease_expires_at = None;
        if let Some(error) = delivery_error {
            updated.last_error = Some(error);
            if updated.attempt >= updated.max_attempts {
                updated.state = OutboxState::DeadLetter;
                updated.completed_at = Some(now);
                updated.next_attempt_at = None;
            } else {
                updated.state = OutboxState::RetryWait;
                updated.next_attempt_at =
                    Some(now + chrono::Duration::seconds(2_i64.pow(updated.attempt.min(8))));
            }
        } else {
            updated.state = OutboxState::Delivered;
            updated.completed_at = Some(now);
            updated.next_attempt_at = None;
            updated.last_error = None;
        }
        let changed = transaction.execute(
            "UPDATE outbox_events
                SET state=?1,completed_at=?2,lease_owner=NULL,lease_expires_at=NULL,
                    next_attempt_at=?3,last_error=?4
              WHERE tenant_id=?5 AND event_id=?6 AND state=?7 AND fencing_token=?8
                AND lease_owner=?9 AND lease_expires_at>?10",
            params![
                enum_json(&updated.state)?,
                updated.completed_at.map(|value| value.to_rfc3339()),
                updated.next_attempt_at.map(|value| value.to_rfc3339()),
                updated.last_error,
                updated.tenant_id,
                updated.event_id,
                enum_json(&OutboxState::Running)?,
                expected_fence as i64,
                expected_owner,
                now.to_rfc3339(),
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict(
                "outbox completion lost compare-and-swap".into(),
            ));
        }
        insert_audit_tx(
            &transaction,
            &AuditEvent {
                event_id: crate::domain::new_id("audit"),
                tenant_id: updated.tenant_id.clone(),
                actor_id: expected_owner.into(),
                action: match updated.state {
                    OutboxState::Delivered => "outbox.delivered",
                    OutboxState::RetryWait => "outbox.retry_scheduled",
                    OutboxState::DeadLetter => "outbox.dead_letter",
                    _ => "outbox.delivery_state",
                }
                .into(),
                target_type: "outbox_event".into(),
                target_id: updated.event_id.clone(),
                request_id: None,
                atxn_id: None,
                outcome: if updated.state == OutboxState::Delivered {
                    "pass".into()
                } else {
                    "warning".into()
                },
                policy_epoch: 0,
                details: serde_json::json!({
                    "attempt": updated.attempt,
                    "fencing_token": updated.fencing_token,
                    "state": updated.state,
                    "last_error": updated.last_error,
                }),
                created_at: now,
            },
        )?;
        transaction.commit()?;
        Ok(updated)
    }

    pub fn enqueue_job(&self, job: &Job) -> Result<Job> {
        validate_initial_job(job)?;
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = enqueue_job_tx(&database, job)?;
        database.commit()?;
        Ok(stored)
    }

    pub fn acquire_job(
        &self,
        tenant: &str,
        worker: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<Job>> {
        if tenant.trim().is_empty() || worker.trim().is_empty() {
            return Err(AmosError::Validation(
                "job acquisition requires tenant and worker identity".into(),
            ));
        }
        if lease_until <= now {
            return Err(AmosError::Validation(
                "job lease expiry must be after acquisition time".into(),
            ));
        }
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        loop {
            let body: Option<String> = database
                .query_row(
                    "SELECT body_json FROM jobs
                     WHERE tenant_id=?1 AND (
                        (state IN ('ready','retry_wait') AND next_run_at<=?2)
                        OR (state='running' AND lease_expires_at IS NOT NULL AND lease_expires_at<=?2)
                     )
                     ORDER BY CASE state WHEN 'running' THEN 0 ELSE 1 END, next_run_at, job_id
                     LIMIT 1",
                    params![tenant, now.to_rfc3339()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(body) = body else {
                database.commit()?;
                return Ok(None);
            };
            let mut job: Job = from_json(&body)?;
            let previous_state = job.state;
            let previous_fence = job.fencing_token;
            if previous_state == JobState::Running && job.attempt >= job.max_attempts {
                job.state = JobState::DeadLetter;
                job.lease_owner = None;
                job.lease_expires_at = None;
                job.dead_letter_reason = Some("lease expired after final attempt".into());
                let changed = database.execute(
                    "UPDATE jobs SET state='dead_letter',lease_owner=NULL,lease_expires_at=NULL,body_json=?1
                     WHERE tenant_id=?2 AND job_id=?3 AND state='running' AND fencing_token=?4
                       AND lease_expires_at IS NOT NULL AND lease_expires_at<=?5",
                    params![
                        to_json(&job)?,
                        tenant,
                        job.job_id,
                        previous_fence as i64,
                        now.to_rfc3339(),
                    ],
                )?;
                if changed != 1 {
                    return Err(AmosError::Conflict(format!(
                        "job lease changed while dead-lettering {}",
                        job.job_id
                    )));
                }
                insert_outbox_tx(
                    &database,
                    &new_outbox_event(
                        tenant,
                        "job.dead_lettered",
                        &job.job_id,
                        format!("{}/fence/{previous_fence}/dead-lettered", job.job_id),
                        serde_json::json!({
                            "job_id": job.job_id,
                            "fencing_token": previous_fence,
                            "reason": job.dead_letter_reason,
                        }),
                    ),
                )?;
                continue;
            }

            job.state = JobState::Running;
            job.attempt = job.attempt.checked_add(1).ok_or_else(|| {
                AmosError::Conflict(format!("job attempt counter exhausted for {}", job.job_id))
            })?;
            job.fencing_token = job.fencing_token.checked_add(1).ok_or_else(|| {
                AmosError::Conflict(format!("job fence counter exhausted for {}", job.job_id))
            })?;
            job.lease_owner = Some(worker.into());
            job.lease_expires_at = Some(lease_until);
            job.dead_letter_reason = None;
            let changed = database.execute(
                "UPDATE jobs SET state='running',fencing_token=?1,lease_owner=?2,lease_expires_at=?3,body_json=?4
                 WHERE tenant_id=?5 AND job_id=?6 AND fencing_token=?7 AND (
                    (state IN ('ready','retry_wait') AND next_run_at<=?8)
                    OR (state='running' AND lease_expires_at IS NOT NULL AND lease_expires_at<=?8)
                 )",
                params![
                    job.fencing_token as i64,
                    worker,
                    lease_until.to_rfc3339(),
                    to_json(&job)?,
                    tenant,
                    job.job_id,
                    previous_fence as i64,
                    now.to_rfc3339(),
                ],
            )?;
            if changed != 1 {
                return Err(AmosError::Conflict(format!(
                    "job acquisition compare-and-swap failed for {}",
                    job.job_id
                )));
            }
            insert_outbox_tx(
                &database,
                &new_outbox_event(
                    tenant,
                    "job.acquired",
                    &job.job_id,
                    format!("{}/fence/{}/acquired", job.job_id, job.fencing_token),
                    serde_json::json!({
                        "job_id": job.job_id,
                        "worker": worker,
                        "previous_state": previous_state,
                        "fencing_token": job.fencing_token,
                        "lease_expires_at": lease_until,
                    }),
                ),
            )?;
            database.commit()?;
            return Ok(Some(job));
        }
    }

    pub fn renew_job_lease(
        &self,
        job: &Job,
        expected_fence: u64,
        expected_owner: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let lease_until = job
            .lease_expires_at
            .ok_or_else(|| AmosError::Validation("renewed job requires a lease expiry".into()))?;
        if lease_until <= now {
            return Err(AmosError::Validation(
                "renewed job lease must expire in the future".into(),
            ));
        }
        if job.state != JobState::Running
            || job.fencing_token != expected_fence
            || job.lease_owner.as_deref() != Some(expected_owner)
        {
            return Err(AmosError::Conflict(
                "job lease renewal used a stale handle".into(),
            ));
        }
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = get_job_tx(&database, &job.tenant_id, &job.job_id)?
            .ok_or_else(|| AmosError::NotFound(job.job_id.clone()))?;
        validate_active_job_lease(&current, job, expected_fence, expected_owner, now)?;
        if job.next_run_at != current.next_run_at
            || job.dead_letter_reason != current.dead_letter_reason
            || current
                .lease_expires_at
                .is_none_or(|current_expiry| lease_until <= current_expiry)
        {
            return Err(AmosError::Conflict(
                "job lease renewal must only extend the current lease".into(),
            ));
        }
        let changed = database.execute(
            "UPDATE jobs SET lease_expires_at=?1,body_json=?2
             WHERE tenant_id=?3 AND job_id=?4 AND state='running' AND fencing_token=?5
               AND lease_owner=?6 AND lease_expires_at>?7",
            params![
                lease_until.to_rfc3339(),
                to_json(job)?,
                job.tenant_id,
                job.job_id,
                expected_fence as i64,
                expected_owner,
                now.to_rfc3339(),
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict("job lease is stale or expired".into()));
        }
        insert_outbox_tx(
            &database,
            &new_outbox_event(
                &job.tenant_id,
                "job.lease_renewed",
                &job.job_id,
                format!(
                    "{}/fence/{expected_fence}/lease/{}",
                    job.job_id,
                    lease_until.timestamp_micros()
                ),
                serde_json::json!({
                    "job_id": job.job_id,
                    "worker": expected_owner,
                    "fencing_token": expected_fence,
                    "lease_expires_at": lease_until,
                }),
            ),
        )?;
        database.commit()?;
        Ok(())
    }

    pub fn finish_job(
        &self,
        job: &Job,
        expected_fence: u64,
        expected_owner: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if !matches!(
            job.state,
            JobState::Complete | JobState::RetryWait | JobState::DeadLetter
        ) || job.lease_owner.is_some()
            || job.lease_expires_at.is_some()
        {
            return Err(AmosError::Validation(
                "finished job has an invalid terminal or retry state".into(),
            ));
        }
        let state_fields_valid = match job.state {
            JobState::Complete => job.dead_letter_reason.is_none(),
            JobState::RetryWait => job.dead_letter_reason.is_none() && job.next_run_at > now,
            JobState::DeadLetter => job
                .dead_letter_reason
                .as_deref()
                .is_some_and(|reason| !reason.trim().is_empty()),
            _ => false,
        };
        if !state_fields_valid {
            return Err(AmosError::Validation(
                "finished job has inconsistent retry or dead-letter metadata".into(),
            ));
        }
        let mut connection = self.connection()?;
        let database = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = get_job_tx(&database, &job.tenant_id, &job.job_id)?
            .ok_or_else(|| AmosError::NotFound(job.job_id.clone()))?;
        validate_active_job_lease(&current, job, expected_fence, expected_owner, now)?;
        let changed = database.execute(
            "UPDATE jobs SET state=?1,lease_owner=NULL,lease_expires_at=NULL,next_run_at=?2,body_json=?3
             WHERE tenant_id=?4 AND job_id=?5 AND state='running' AND fencing_token=?6
               AND lease_owner=?7 AND lease_expires_at>?8",
            params![
                enum_json(&job.state)?,
                job.next_run_at.to_rfc3339(),
                to_json(job)?,
                job.tenant_id,
                job.job_id,
                expected_fence as i64,
                expected_owner,
                now.to_rfc3339(),
            ],
        )?;
        if changed != 1 {
            return Err(AmosError::Conflict("job lease is stale or expired".into()));
        }
        let state = enum_json(&job.state)?;
        insert_outbox_tx(
            &database,
            &new_outbox_event(
                &job.tenant_id,
                match job.state {
                    JobState::Complete => "job.completed",
                    JobState::RetryWait => "job.retry_scheduled",
                    JobState::DeadLetter => "job.dead_lettered",
                    _ => unreachable!("validated job completion state"),
                },
                &job.job_id,
                format!(
                    "{}/fence/{expected_fence}/{}",
                    job.job_id,
                    state.replace('_', "-")
                ),
                serde_json::json!({
                    "job_id": job.job_id,
                    "worker": expected_owner,
                    "fencing_token": expected_fence,
                    "state": job.state,
                    "next_run_at": job.next_run_at,
                    "dead_letter_reason": job.dead_letter_reason,
                }),
            ),
        )?;
        database.commit()?;
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

fn reachable_claim_ids(
    connection: &Connection,
    tenant: &str,
    target_type: &str,
    target_id: &str,
    node_quota: usize,
) -> Result<BTreeSet<String>> {
    let mut visited = BTreeSet::new();
    let mut frontier = vec![(target_type.to_string(), target_id.to_string())];
    let mut cursor = 0;
    let mut claims = BTreeSet::new();
    while cursor < frontier.len() {
        let (node_type, node_id) = frontier[cursor].clone();
        cursor += 1;
        if !visited.insert((node_type.clone(), node_id.clone())) {
            continue;
        }
        if visited.len() > node_quota {
            return Err(AmosError::Execution(format!(
                "invalidation traversal node quota exceeded ({node_quota})"
            )));
        }
        let mut statement = connection.prepare(
            "SELECT DISTINCT from_type,from_id
               FROM dependency_edges
              WHERE tenant_id=?1 AND to_type=?2 AND to_id=?3
              ORDER BY from_type,from_id",
        )?;
        let upstream = statement
            .query_map(params![tenant, node_type, node_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for (upstream_type, upstream_id) in upstream {
            if upstream_type == "claim" {
                claims.insert(upstream_id.clone());
            }
            if !visited.contains(&(upstream_type.clone(), upstream_id.clone())) {
                frontier.push((upstream_type, upstream_id));
            }
        }
    }
    Ok(claims)
}

fn validate_initial_transaction(transaction: &AnalyticalTransaction) -> Result<()> {
    if transaction.tenant_id.trim().is_empty()
        || transaction.atxn_id.trim().is_empty()
        || transaction.request_id.trim().is_empty()
        || transaction.idempotency_key.trim().is_empty()
        || transaction.request_hash.trim().is_empty()
        || transaction.subject_id.trim().is_empty()
    {
        return Err(AmosError::Validation(
            "A-TXN admission requires tenant, transaction, request, idempotency, hash, and subject identifiers"
                .into(),
        ));
    }
    if transaction.state != AtxnState::Admitted
        || transaction.state_seq != 0
        || transaction.terminal
        || transaction.outcome.is_some()
    {
        return Err(AmosError::Validation(
            "new A-TXN must begin in admitted state at sequence zero".into(),
        ));
    }
    Ok(())
}

fn validate_transaction_checkpoint(
    current: &AnalyticalTransaction,
    candidate: &AnalyticalTransaction,
) -> Result<()> {
    if current.terminal {
        return Err(AmosError::Conflict(
            "terminal A-TXN records are immutable".into(),
        ));
    }
    let fixed_fields_match = current.tenant_id == candidate.tenant_id
        && current.atxn_id == candidate.atxn_id
        && current.request_id == candidate.request_id
        && current.idempotency_key == candidate.idempotency_key
        && current.request_hash == candidate.request_hash
        && current.subject_id == candidate.subject_id
        && current.request == candidate.request
        && current.task_type == candidate.task_type
        && current.task_version == candidate.task_version
        && current.risk_class == candidate.risk_class
        && current.budgets == candidate.budgets
        && current.policy_epoch == candidate.policy_epoch
        && current.created_at == candidate.created_at
        && current.state == candidate.state
        && current.state_seq == candidate.state_seq
        && current.terminal == candidate.terminal
        && current.outcome == candidate.outcome;
    if !fixed_fields_match {
        return Err(AmosError::Conflict(
            "A-TXN checkpoint attempted to mutate fixed admission or lifecycle fields".into(),
        ));
    }
    if candidate.updated_at < current.updated_at {
        return Err(AmosError::Conflict(
            "A-TXN checkpoint timestamp moved backwards".into(),
        ));
    }
    if current
        .source_versions
        .iter()
        .any(|(source, version)| candidate.source_versions.get(source) != Some(version))
    {
        return Err(AmosError::Conflict(
            "A-TXN checkpoint attempted to replace an observed source version".into(),
        ));
    }
    Ok(())
}

fn ensure_transaction_snapshot(
    connection: &Connection,
    expected: &AnalyticalTransaction,
) -> Result<()> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body_json FROM atxn_transactions WHERE tenant_id=?1 AND atxn_id=?2",
            params![expected.tenant_id, expected.atxn_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: AnalyticalTransaction = body
        .map(|body| from_json(&body))
        .transpose()?
        .ok_or_else(|| AmosError::NotFound(expected.atxn_id.clone()))?;
    if current != *expected {
        return Err(AmosError::Conflict(format!(
            "A-TXN snapshot changed before atomic commit for {}",
            expected.atxn_id
        )));
    }
    Ok(())
}

fn ensure_artifact_snapshot(connection: &Connection, expected: &Artifact) -> Result<()> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body_json FROM artifacts WHERE tenant_id=?1 AND artifact_id=?2",
            params![expected.tenant_id, expected.artifact_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: Artifact = body
        .map(|body| from_json(&body))
        .transpose()?
        .ok_or_else(|| AmosError::NotFound(expected.artifact_id.clone()))?;
    if current != *expected {
        return Err(AmosError::Conflict(format!(
            "artifact snapshot changed before atomic publication for {}",
            expected.artifact_id
        )));
    }
    Ok(())
}

fn ensure_claim_snapshot(connection: &Connection, expected: &Claim) -> Result<()> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body_json FROM claims WHERE tenant_id=?1 AND claim_id=?2",
            params![expected.tenant_id, expected.claim_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: Claim = body
        .map(|body| from_json(&body))
        .transpose()?
        .ok_or_else(|| AmosError::NotFound(expected.claim_id.clone()))?;
    if current != *expected {
        return Err(AmosError::Conflict(format!(
            "claim snapshot changed before atomic commit for {}",
            expected.claim_id
        )));
    }
    Ok(())
}

fn validate_review_bundle(
    review: &Review,
    artifact: &Artifact,
    expected_claims: &[Claim],
    updated_claims: &[Claim],
    feedback: &MemoryObject,
    audit: &AuditEvent,
    revalidation_job: &Job,
) -> Result<()> {
    let target_ids = review.claim_ids.iter().collect::<BTreeSet<_>>();
    let expected_by_id = expected_claims
        .iter()
        .map(|claim| (claim.claim_id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let updated_by_id = updated_claims
        .iter()
        .map(|claim| (claim.claim_id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    if review.tenant_id.trim().is_empty()
        || review.artifact_id.trim().is_empty()
        || review.idempotency_key.trim().is_empty()
        || review.request_hash.trim().is_empty()
        || review.request_hash != calculated_review_request_hash(review)?
        || review.reviewer_id.trim().is_empty()
        || review.comment.trim().is_empty()
        || review.original_artifact_mutated
        || review.claim_ids.is_empty()
        || target_ids.len() != review.claim_ids.len()
        || expected_claims.is_empty()
        || expected_claims.len() != updated_claims.len()
        || expected_by_id.len() != expected_claims.len()
        || updated_by_id.len() != updated_claims.len()
        || expected_by_id.keys().ne(updated_by_id.keys())
        || artifact.tenant_id != review.tenant_id
        || artifact.artifact_id != review.artifact_id
        || target_ids
            .iter()
            .any(|claim_id| !expected_by_id.contains_key(claim_id.as_str()))
    {
        return Err(AmosError::Validation(
            "review bundle contains invalid identity, idempotency, artifact, or claim data".into(),
        ));
    }
    if matches!(review.decision, crate::domain::ReviewDecision::Correct)
        != review.correction.is_some()
    {
        return Err(AmosError::Validation(
            "only a correction decision may carry correction data, and it must carry it".into(),
        ));
    }
    let expected_review_state = match review.decision {
        crate::domain::ReviewDecision::Approve => crate::domain::ReviewState::Approved,
        crate::domain::ReviewDecision::Reject => crate::domain::ReviewState::Rejected,
        crate::domain::ReviewDecision::Correct => crate::domain::ReviewState::Corrected,
    };
    for (claim_id, expected) in &expected_by_id {
        let updated = updated_by_id
            .get(claim_id)
            .ok_or_else(|| AmosError::Validation("review claim set changed".into()))?;
        if !same_claim_evidence(expected, updated)
            || expected.publication_validity != updated.publication_validity
            || expected.semantic_validity != updated.semantic_validity
            || expected.policy_visibility != updated.policy_visibility
            || expected.replay_availability != updated.replay_availability
            || expected.supersession_state != updated.supersession_state
            || (target_ids.contains(&expected.claim_id)
                && updated.review_state != expected_review_state)
            || (!target_ids.contains(&expected.claim_id) && *expected != *updated)
        {
            return Err(AmosError::Validation(format!(
                "review attempted an unauthorized mutation of claim {claim_id}"
            )));
        }
    }
    if feedback.tenant_id != review.tenant_id
        || feedback.memory_type != crate::domain::MemoryType::Feedback
        || feedback.source_id != "review"
        || feedback.source_version != review.review_id
        || feedback.provenance_ref.as_deref() != Some(review.artifact_id.as_str())
        || feedback.content_hash != crate::domain::content_hash(&feedback.content)?
        || audit.tenant_id != review.tenant_id
        || audit.actor_id != review.reviewer_id
        || audit.action != "review.append"
        || audit.target_type != "artifact"
        || audit.target_id != review.artifact_id
        || revalidation_job.tenant_id != review.tenant_id
        || revalidation_job.job_type != "claim.revalidate"
        || revalidation_job
            .payload
            .get("artifact_id")
            .and_then(serde_json::Value::as_str)
            != Some(review.artifact_id.as_str())
    {
        return Err(AmosError::Validation(
            "review feedback, audit, or revalidation job is inconsistent".into(),
        ));
    }
    validate_initial_job(revalidation_job)
}

fn calculated_review_request_hash(review: &Review) -> Result<String> {
    crate::domain::content_hash(&serde_json::json!({
        "tenant_id": review.tenant_id,
        "artifact_id": review.artifact_id,
        "claim_ids": review.claim_ids,
        "reviewer_id": review.reviewer_id,
        "decision": review.decision,
        "comment": review.comment,
        "correction": review.correction,
        "authority": review.authority,
    }))
}

fn validate_claim_validity_batch(
    expected_claims: &[Claim],
    updated_claims: &[Claim],
    audit: &AuditEvent,
    cause: &str,
) -> Result<()> {
    if expected_claims.is_empty()
        || expected_claims.len() != updated_claims.len()
        || cause.trim().is_empty()
    {
        return Err(AmosError::Validation(
            "claim validity commit requires equal non-empty claim sets and a cause".into(),
        ));
    }
    let expected_by_id = expected_claims
        .iter()
        .map(|claim| (claim.claim_id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let updated_by_id = updated_claims
        .iter()
        .map(|claim| (claim.claim_id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let artifact_id = expected_claims[0].artifact_id.as_str();
    let tenant_id = expected_claims[0].tenant_id.as_str();
    if expected_by_id.len() != expected_claims.len()
        || updated_by_id.len() != updated_claims.len()
        || expected_by_id.keys().ne(updated_by_id.keys())
        || audit.tenant_id != tenant_id
        || audit.target_type != "artifact"
        || audit.target_id != artifact_id
    {
        return Err(AmosError::Validation(
            "claim validity commit contains inconsistent claim or audit identity".into(),
        ));
    }
    for (claim_id, expected) in expected_by_id {
        let updated = updated_by_id
            .get(claim_id)
            .ok_or_else(|| AmosError::Validation("claim validity set changed".into()))?;
        if expected.tenant_id != tenant_id
            || expected.artifact_id != artifact_id
            || !same_claim_evidence(expected, updated)
            || expected.publication_validity != updated.publication_validity
            || expected.review_state != updated.review_state
            || !valid_semantic_transition(expected.semantic_validity, updated.semantic_validity)
            || !valid_replay_transition(expected.replay_availability, updated.replay_availability)
            || !valid_supersession_transition(
                expected.supersession_state,
                updated.supersession_state,
            )
        {
            return Err(AmosError::Validation(format!(
                "validity commit attempted to rewrite evidence for claim {claim_id}"
            )));
        }
    }
    Ok(())
}

fn valid_semantic_transition(
    current: crate::domain::SemanticValidity,
    next: crate::domain::SemanticValidity,
) -> bool {
    use crate::domain::SemanticValidity::{Current, Invalid, PendingRevalidation, Stale};
    current == next
        || matches!(
            (current, next),
            (Current, PendingRevalidation | Stale | Invalid)
                | (PendingRevalidation, Current | Stale | Invalid)
                | (Stale, PendingRevalidation | Invalid)
        )
}

fn valid_replay_transition(
    current: crate::domain::ReplayAvailability,
    next: crate::domain::ReplayAvailability,
) -> bool {
    use crate::domain::ReplayAvailability::{Degraded, Expired};
    current == next || matches!(next, Degraded | Expired) && current != Expired
}

fn valid_supersession_transition(
    current: crate::domain::SupersessionState,
    next: crate::domain::SupersessionState,
) -> bool {
    use crate::domain::SupersessionState::{Active, Superseded, Tombstoned};
    current == next
        || matches!(
            (current, next),
            (Active, Superseded | Tombstoned) | (Superseded, Tombstoned)
        )
}

fn same_claim_evidence(left: &Claim, right: &Claim) -> bool {
    left.tenant_id == right.tenant_id
        && left.claim_id == right.claim_id
        && left.artifact_id == right.artifact_id
        && left.claim_type == right.claim_type
        && left.text == right.text
        && left.payload == right.payload
        && left.risk_class == right.risk_class
        && left.support_execution_ids == right.support_execution_ids
        && left.verification_ids == right.verification_ids
}

fn update_claim_validity_tx(connection: &Connection, updated: &Claim) -> Result<usize> {
    connection
        .execute(
            "UPDATE claims
                SET publication_validity=?1, semantic_validity=?2, policy_visibility=?3,
                    replay_availability=?4, review_state=?5, supersession_state=?6,
                    validity_seq=validity_seq+1, body_json=?7
              WHERE tenant_id=?8 AND claim_id=?9",
            params![
                enum_json(&updated.publication_validity)?,
                enum_json(&updated.semantic_validity)?,
                enum_json(&updated.policy_visibility)?,
                enum_json(&updated.replay_availability)?,
                enum_json(&updated.review_state)?,
                enum_json(&updated.supersession_state)?,
                to_json(updated)?,
                updated.tenant_id,
                updated.claim_id
            ],
        )
        .map_err(Into::into)
}

fn claim_validity_json(claim: &Claim) -> serde_json::Value {
    serde_json::json!({
        "publication_validity": claim.publication_validity,
        "semantic_validity": claim.semantic_validity,
        "policy_visibility": claim.policy_visibility,
        "replay_availability": claim.replay_availability,
        "review_state": claim.review_state,
        "supersession_state": claim.supersession_state,
    })
}

fn validate_evidence_bundle(
    atxn: &AnalyticalTransaction,
    artifact: &Artifact,
    claims: &[Claim],
    edges: &[DependencyEdge],
    package: &ReplayPackage,
    audit: &AuditEvent,
) -> Result<()> {
    if artifact.tenant_id != atxn.tenant_id
        || artifact.atxn_id != atxn.atxn_id
        || package.tenant_id != atxn.tenant_id
        || package.artifact_id != artifact.artifact_id
        || package.expected_artifact_hash != artifact.content_hash
        || audit.tenant_id != atxn.tenant_id
        || audit.atxn_id.as_deref() != Some(atxn.atxn_id.as_str())
        || audit.target_type != "artifact"
        || audit.target_id != artifact.artifact_id
    {
        return Err(AmosError::Validation(
            "evidence bundle contains inconsistent tenant, transaction, artifact, or audit identity"
                .into(),
        ));
    }
    let claim_ids = claims
        .iter()
        .map(|claim| claim.claim_id.as_str())
        .collect::<BTreeSet<_>>();
    if claim_ids.len() != claims.len()
        || claims.iter().any(|claim| {
            claim.tenant_id != atxn.tenant_id || claim.artifact_id != artifact.artifact_id
        })
        || edges.iter().any(|edge| {
            edge.tenant_id != atxn.tenant_id
                || edge.created_by_atxn != atxn.atxn_id
                || edge.from.endpoint_type != "claim"
                || !claim_ids.contains(edge.from.id.as_str())
        })
    {
        return Err(AmosError::Validation(
            "evidence bundle contains duplicate or cross-resource claims and dependencies".into(),
        ));
    }
    Ok(())
}

fn validate_publication_bundle(
    atxn: &AnalyticalTransaction,
    artifact: &Artifact,
    claims: &[Claim],
    audit: &AuditEvent,
) -> Result<()> {
    if artifact.tenant_id != atxn.tenant_id
        || artifact.atxn_id != atxn.atxn_id
        || claims.iter().any(|claim| {
            claim.tenant_id != atxn.tenant_id || claim.artifact_id != artifact.artifact_id
        })
        || audit.tenant_id != atxn.tenant_id
        || audit.atxn_id.as_deref() != Some(atxn.atxn_id.as_str())
        || audit.target_type != "artifact"
        || audit.target_id != artifact.artifact_id
        || artifact.publication_validity != PublicationValidity::Draft
    {
        return Err(AmosError::Validation(
            "publication bundle contains inconsistent tenant, transaction, artifact, claim, or audit identity"
                .into(),
        ));
    }
    Ok(())
}

fn validate_initial_job(job: &Job) -> Result<()> {
    if job.tenant_id.trim().is_empty()
        || job.job_id.trim().is_empty()
        || job.job_type.trim().is_empty()
        || job.idempotency_key.trim().is_empty()
    {
        return Err(AmosError::Validation(
            "job enqueue requires tenant, job, type, and idempotency identifiers".into(),
        ));
    }
    if job.state != JobState::Ready
        || job.attempt != 0
        || job.max_attempts == 0
        || job.fencing_token != 0
        || job.lease_owner.is_some()
        || job.lease_expires_at.is_some()
        || job.dead_letter_reason.is_some()
    {
        return Err(AmosError::Validation(
            "new job must be an unleased ready job with a positive attempt budget".into(),
        ));
    }
    Ok(())
}

fn same_execution_effect(existing: &ExecutionRecord, candidate: &ExecutionRecord) -> bool {
    existing.tenant_id == candidate.tenant_id
        && existing.atxn_id == candidate.atxn_id
        && existing.plan_id == candidate.plan_id
        && existing.step_id == candidate.step_id
        && existing.tool == candidate.tool
        && existing.tool_version == candidate.tool_version
        && existing.parameters_hash == candidate.parameters_hash
        && existing.input_versions == candidate.input_versions
        && existing.output == candidate.output
        && existing.output_hash == candidate.output_hash
        && existing.row_count == candidate.row_count
        && existing.byte_count == candidate.byte_count
        && existing.fencing_token == candidate.fencing_token
        && existing.status == candidate.status
}

fn same_verification_effect(existing: &VerificationRecord, candidate: &VerificationRecord) -> bool {
    existing.tenant_id == candidate.tenant_id
        && existing.atxn_id == candidate.atxn_id
        && existing.execution_id == candidate.execution_id
        && existing.verifier_profile == candidate.verifier_profile
        && existing.profile_version == candidate.profile_version
        && existing.outcome == candidate.outcome
        && existing.checks == candidate.checks
        && existing.warnings == candidate.warnings
        && existing.errors == candidate.errors
        && existing.permitted_repair == candidate.permitted_repair
        && existing.input_hash == candidate.input_hash
}

fn same_job_request(existing: &Job, candidate: &Job) -> bool {
    existing.tenant_id == candidate.tenant_id
        && existing.idempotency_key == candidate.idempotency_key
        && existing.job_type == candidate.job_type
        && existing.payload == candidate.payload
        && existing.max_attempts == candidate.max_attempts
}

fn get_job_tx(connection: &Connection, tenant: &str, job_id: &str) -> Result<Option<Job>> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body_json FROM jobs WHERE tenant_id=?1 AND job_id=?2",
            params![tenant, job_id],
            |row| row.get(0),
        )
        .optional()?;
    body.map(|body| from_json(&body)).transpose()
}

fn get_outbox_event_tx(
    connection: &Connection,
    tenant: &str,
    event_id: &str,
) -> Result<Option<OutboxEvent>> {
    let row = connection
        .query_row(
            "SELECT tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,
                    created_at,completed_at,state,attempt,max_attempts,fencing_token,
                    lease_owner,lease_expires_at,next_attempt_at,last_error
               FROM outbox_events WHERE tenant_id=?1 AND event_id=?2",
            params![tenant, event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, u32>(9)?,
                    row.get::<_, u32>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            tenant_id,
            event_id,
            event_type,
            aggregate_id,
            idempotency_key,
            payload_json,
            created_at,
            completed_at,
            state,
            attempt,
            max_attempts,
            fencing_token,
            lease_owner,
            lease_expires_at,
            next_attempt_at,
            last_error,
        )| {
            Ok(OutboxEvent {
                tenant_id,
                event_id,
                event_type,
                aggregate_id,
                idempotency_key,
                payload: from_json(&payload_json)?,
                created_at: parse_timestamp(&created_at)?,
                completed_at: completed_at.as_deref().map(parse_timestamp).transpose()?,
                state: parse_outbox_state(&state)?,
                attempt,
                max_attempts,
                fencing_token: u64::try_from(fencing_token)
                    .map_err(|_| AmosError::Storage("negative outbox fencing token".into()))?,
                lease_owner,
                lease_expires_at: lease_expires_at
                    .as_deref()
                    .map(parse_timestamp)
                    .transpose()?,
                next_attempt_at: next_attempt_at
                    .as_deref()
                    .map(parse_timestamp)
                    .transpose()?,
                last_error,
            })
        },
    )
    .transpose()
}

fn validate_active_job_lease(
    current: &Job,
    candidate: &Job,
    expected_fence: u64,
    expected_owner: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    let fixed_fields_match = current.tenant_id == candidate.tenant_id
        && current.job_id == candidate.job_id
        && current.job_type == candidate.job_type
        && current.payload == candidate.payload
        && current.idempotency_key == candidate.idempotency_key
        && current.attempt == candidate.attempt
        && current.max_attempts == candidate.max_attempts
        && current.fencing_token == candidate.fencing_token;
    if !fixed_fields_match {
        return Err(AmosError::Conflict(
            "job mutation attempted to change fixed identity, payload, attempt, or fence fields"
                .into(),
        ));
    }
    if current.state != JobState::Running
        || current.fencing_token != expected_fence
        || current.lease_owner.as_deref() != Some(expected_owner)
        || current
            .lease_expires_at
            .is_none_or(|expires_at| expires_at <= now)
    {
        return Err(AmosError::Conflict("job lease is stale or expired".into()));
    }
    Ok(())
}

fn new_outbox_event(
    tenant: &str,
    event_type: &str,
    aggregate_id: &str,
    idempotency_key: String,
    payload: serde_json::Value,
) -> OutboxEvent {
    OutboxEvent {
        tenant_id: tenant.into(),
        event_id: crate::domain::new_id("evt"),
        event_type: event_type.into(),
        aggregate_id: aggregate_id.into(),
        idempotency_key,
        payload,
        created_at: Utc::now(),
        completed_at: None,
        state: OutboxState::Ready,
        attempt: 0,
        max_attempts: 8,
        fencing_token: 0,
        lease_owner: None,
        lease_expires_at: None,
        next_attempt_at: Some(Utc::now()),
        last_error: None,
    }
}

fn insert_outbox_tx(connection: &Connection, event: &OutboxEvent) -> Result<()> {
    connection.execute(
        "INSERT INTO outbox_events
         (tenant_id,event_id,event_type,aggregate_id,idempotency_key,payload_json,
          created_at,completed_at,state,attempt,max_attempts,fencing_token,
          lease_owner,lease_expires_at,next_attempt_at,last_error)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            event.tenant_id,
            event.event_id,
            event.event_type,
            event.aggregate_id,
            event.idempotency_key,
            to_json(&event.payload)?,
            event.created_at.to_rfc3339(),
            event.completed_at.map(|value| value.to_rfc3339()),
            enum_json(&event.state)?,
            event.attempt,
            event.max_attempts,
            event.fencing_token as i64,
            event.lease_owner,
            event.lease_expires_at.map(|value| value.to_rfc3339()),
            event.next_attempt_at.map(|value| value.to_rfc3339()),
            event.last_error,
        ],
    )?;
    Ok(())
}

fn enqueue_job_tx(connection: &Connection, job: &Job) -> Result<Job> {
    validate_initial_job(job)?;
    let existing: Option<String> = connection
        .query_row(
            "SELECT body_json FROM jobs WHERE tenant_id=?1 AND idempotency_key=?2",
            params![job.tenant_id, job.idempotency_key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        let existing: Job = from_json(&existing)?;
        return if same_job_request(&existing, job) {
            Ok(existing)
        } else {
            Err(AmosError::IdempotencyConflict(job.idempotency_key.clone()))
        };
    }
    connection.execute(
        "INSERT INTO jobs
         (tenant_id,job_id,job_type,idempotency_key,state,fencing_token,lease_owner,
          lease_expires_at,next_run_at,body_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            job.tenant_id,
            job.job_id,
            job.job_type,
            job.idempotency_key,
            enum_json(&job.state)?,
            job.fencing_token as i64,
            job.lease_owner,
            job.lease_expires_at.map(|value| value.to_rfc3339()),
            job.next_run_at.to_rfc3339(),
            to_json(job)?
        ],
    )?;
    insert_outbox_tx(
        connection,
        &new_outbox_event(
            &job.tenant_id,
            "job.enqueued",
            &job.job_id,
            format!("{}/enqueued", job.job_id),
            serde_json::json!({
                "job_id": job.job_id,
                "job_type": job.job_type,
                "state": job.state,
            }),
        ),
    )?;
    Ok(job.clone())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AmosError::Storage(format!("invalid stored timestamp: {error}")))
}

fn parse_outbox_state(value: &str) -> Result<OutboxState> {
    match value.trim_matches('"') {
        "ready" => Ok(OutboxState::Ready),
        "running" => Ok(OutboxState::Running),
        "retry_wait" => Ok(OutboxState::RetryWait),
        "delivered" => Ok(OutboxState::Delivered),
        "dead_letter" => Ok(OutboxState::DeadLetter),
        other => Err(AmosError::Serialization(format!(
            "unknown outbox state {other}"
        ))),
    }
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if !table
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
        || !column
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(AmosError::Validation(
            "schema migration identifiers must be alphanumeric".into(),
        ));
    }
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn record_migration(
    connection: &Connection,
    version: u32,
    name: &str,
    contract: &str,
) -> Result<()> {
    let checksum = content_hash(&(version, name, contract))?;
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT name,checksum FROM schema_migrations WHERE version=?1",
            [version],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((existing_name, existing_checksum)) = existing {
        if existing_name != name || existing_checksum != checksum {
            return Err(AmosError::Storage(format!(
                "schema migration {version} checksum mismatch"
            )));
        }
        return Ok(());
    }
    connection.execute(
        "INSERT INTO schema_migrations(version,name,checksum,applied_at)
         VALUES (?1,?2,?3,?4)",
        params![version, name, checksum, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn insert_memory_tx(transaction: &rusqlite::Transaction<'_>, object: &MemoryObject) -> Result<()> {
    insert_memory_tx_with_json(
        transaction,
        object,
        &to_json(&object.permissions)?,
        &to_json(object)?,
    )
}

fn insert_memory_tx_with_json(
    transaction: &rusqlite::Transaction<'_>,
    object: &MemoryObject,
    permissions_json: &str,
    body_json: &str,
) -> Result<()> {
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
            permissions_json,
            object.sensitivity,
            object.version,
            enum_json(&object.status)?,
            object.superseded_by,
            object.content_hash,
            object.governing,
            body_json
        ],
    )?;
    let rowid = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO memory_fts(
            rowid,tenant_id,object_id,logical_key,summary,content
         ) VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            rowid,
            object.tenant_id,
            object.object_id,
            object.logical_key,
            object.summary,
            serde_json::to_string(&object.content)?,
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
    use std::{
        collections::BTreeMap,
        sync::{Arc, Barrier},
        thread,
    };

    use chrono::Duration;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::domain::{Authority, Budgets, MemoryType, RiskClass};

    fn admitted_transaction(
        atxn_id: &str,
        request_id: &str,
        idempotency_key: &str,
        request_hash: &str,
    ) -> AnalyticalTransaction {
        let now = Utc::now();
        AnalyticalTransaction {
            tenant_id: "tenant".into(),
            atxn_id: atxn_id.into(),
            request_id: request_id.into(),
            idempotency_key: idempotency_key.into(),
            request_hash: request_hash.into(),
            subject_id: "analyst".into(),
            request: "Investigate payment failures".into(),
            task_type: "payment_health_review".into(),
            task_version: 1,
            risk_class: RiskClass::MaterialInternal,
            budgets: Budgets::default(),
            policy_epoch: 1,
            source_versions: BTreeMap::new(),
            state: AtxnState::Admitted,
            state_seq: 0,
            terminal: false,
            outcome: None,
            warnings: vec![],
            errors: vec![],
            created_at: now,
            updated_at: now,
        }
    }

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
        )
        .unwrap();
        store.write_memory(&first).unwrap();
        store.write_memory(&first).unwrap();
        let mut changed = first.clone();
        changed.object_id = crate::domain::new_id("mem");
        changed.content = json!({"v":2});
        changed.content_hash = crate::domain::content_hash(&changed.content).unwrap();
        assert!(matches!(
            store.write_memory(&changed),
            Err(AmosError::Conflict(_))
        ));
    }

    #[test]
    fn retention_and_erasure_are_atomic_idempotent_and_legal_hold_safe() {
        let store = Store::in_memory().unwrap();
        let object = MemoryObject::new(
            "tenant",
            "document:privacy",
            MemoryType::Document,
            "privacy-sensitive document",
            json!({"subject":"customer-17"}),
            "upload",
            "v1",
            Authority::OwnerApproved,
        )
        .unwrap();
        store.write_memory(&object).unwrap();
        let now = Utc::now();
        let mut retention = RetentionRecord {
            tenant_id: "tenant".into(),
            target_type: "memory".into(),
            target_id: object.object_id.clone(),
            retained_until: now - chrono::Duration::seconds(1),
            legal_hold: true,
            reason: "litigation".into(),
            updated_by: "admin".into(),
            updated_at: now,
        };
        store.set_retention(&retention, "retention-hold").unwrap();
        assert!(matches!(
            store.erase_memory(
                "tenant",
                &object.object_id,
                "admin",
                "erase-held",
                now
            ),
            Err(AmosError::Conflict(message)) if message.contains("legal hold")
        ));

        retention.legal_hold = false;
        retention.reason = "approved erasure".into();
        retention.updated_at = now + chrono::Duration::seconds(1);
        store
            .set_retention(&retention, "retention-release")
            .unwrap();
        let receipt = store
            .erase_memory("tenant", &object.object_id, "admin", "erase-approved", now)
            .unwrap();
        assert_eq!(receipt.erased_content_hash, object.content_hash);
        assert!(
            store
                .get_memory("tenant", &object.object_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .erase_memory("tenant", &object.object_id, "admin", "erase-approved", now,)
                .unwrap(),
            receipt
        );
        assert_eq!(
            store
                .list_audit("tenant", 20)
                .unwrap()
                .iter()
                .filter(|event| event.action == "memory.erase")
                .count(),
            1
        );
        assert_eq!(
            store
                .list_outbox("tenant", 20)
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "memory.erased")
                .count(),
            1
        );
    }

    #[test]
    fn schema_upgrade_backfills_queryable_claim_validity_dimensions() {
        let root = TempDir::new().unwrap();
        let database = root.path().join("legacy.sqlite");
        let claim = Claim {
            tenant_id: "tenant".into(),
            claim_id: "claim_legacy".into(),
            artifact_id: "artifact_legacy".into(),
            claim_type: "metric".into(),
            text: "A legacy claim".into(),
            payload: json!({"value":1}),
            risk_class: RiskClass::MaterialInternal,
            support_execution_ids: vec![],
            verification_ids: vec![],
            publication_validity: PublicationValidity::ValidAtPublication,
            semantic_validity: crate::domain::SemanticValidity::Stale,
            policy_visibility: crate::domain::PolicyVisibility::Redacted,
            replay_availability: crate::domain::ReplayAvailability::Degraded,
            review_state: crate::domain::ReviewState::Approved,
            supersession_state: crate::domain::SupersessionState::Active,
        };
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE claims (
                    tenant_id TEXT NOT NULL, claim_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                    semantic_validity TEXT NOT NULL, policy_visibility TEXT NOT NULL,
                    review_state TEXT NOT NULL, supersession_state TEXT NOT NULL,
                    body_json TEXT NOT NULL, PRIMARY KEY (tenant_id, claim_id)
                );
                CREATE TABLE reviews (
                    tenant_id TEXT NOT NULL, review_id TEXT NOT NULL, artifact_id TEXT NOT NULL,
                    reviewer_id TEXT NOT NULL, decision TEXT NOT NULL, body_json TEXT NOT NULL,
                    PRIMARY KEY (tenant_id, review_id)
                );
                "#,
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO claims
                 (tenant_id,claim_id,artifact_id,semantic_validity,policy_visibility,
                  review_state,supersession_state,body_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    claim.tenant_id,
                    claim.claim_id,
                    claim.artifact_id,
                    enum_json(&claim.semantic_validity).unwrap(),
                    enum_json(&claim.policy_visibility).unwrap(),
                    enum_json(&claim.review_state).unwrap(),
                    enum_json(&claim.supersession_state).unwrap(),
                    to_json(&claim).unwrap()
                ],
            )
            .unwrap();
        drop(connection);

        let store = Store::open(&database).unwrap();
        assert_eq!(
            store.get_claim("tenant", "claim_legacy").unwrap().unwrap(),
            claim
        );
        let connection = store.connection().unwrap();
        let dimensions = connection
            .query_row(
                "SELECT publication_validity,replay_availability,validity_seq
                   FROM claims WHERE tenant_id='tenant' AND claim_id='claim_legacy'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            dimensions,
            ("valid_at_publication".into(), "degraded".into(), 0)
        );
        let migration_count = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get::<_, u32>(0)
            })
            .unwrap();
        drop(connection);
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(migration_count, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_open_fails_closed_for_future_or_tampered_migrations() {
        let root = TempDir::new().unwrap();
        let future = root.path().join("future.sqlite");
        let connection = Connection::open(&future).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations(
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE,
                    checksum TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                 );
                 INSERT INTO schema_migrations VALUES(999,'future','hash','now');",
            )
            .unwrap();
        drop(connection);
        assert!(matches!(Store::open(&future), Err(AmosError::Storage(_))));

        let tampered = root.path().join("tampered.sqlite");
        let store = Store::open(&tampered).unwrap();
        drop(store);
        let connection = Connection::open(&tampered).unwrap();
        connection
            .execute(
                "UPDATE schema_migrations SET checksum='tampered' WHERE version=3",
                [],
            )
            .unwrap();
        drop(connection);
        assert!(matches!(
            Store::open(&tampered),
            Err(AmosError::Storage(message)) if message.contains("checksum mismatch")
        ));
    }

    #[test]
    fn invalidation_is_bounded_and_enqueues_a_continuation_page() {
        let store = Store::in_memory().unwrap();
        {
            let connection = store.connection().unwrap();
            for suffix in ["a", "b", "c", "d"] {
                let claim = Claim {
                    tenant_id: "tenant".into(),
                    claim_id: format!("claim_{suffix}"),
                    artifact_id: "artifact".into(),
                    claim_type: "metric".into(),
                    text: format!("claim {suffix}"),
                    payload: json!({"value":suffix}),
                    risk_class: RiskClass::MaterialInternal,
                    support_execution_ids: vec![],
                    verification_ids: vec![],
                    publication_validity: PublicationValidity::Draft,
                    semantic_validity: crate::domain::SemanticValidity::Current,
                    policy_visibility: crate::domain::PolicyVisibility::Allowed,
                    replay_availability: crate::domain::ReplayAvailability::Level2,
                    review_state: crate::domain::ReviewState::Verified,
                    supersession_state: crate::domain::SupersessionState::Active,
                };
                connection
                    .execute(
                        "INSERT INTO claims
                         (tenant_id,claim_id,artifact_id,publication_validity,
                          semantic_validity,policy_visibility,replay_availability,
                          review_state,supersession_state,validity_seq,body_json)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10)",
                        params![
                            claim.tenant_id,
                            claim.claim_id,
                            claim.artifact_id,
                            enum_json(&claim.publication_validity).unwrap(),
                            enum_json(&claim.semantic_validity).unwrap(),
                            enum_json(&claim.policy_visibility).unwrap(),
                            enum_json(&claim.replay_availability).unwrap(),
                            enum_json(&claim.review_state).unwrap(),
                            enum_json(&claim.supersession_state).unwrap(),
                            to_json(&claim).unwrap()
                        ],
                    )
                    .unwrap();
                if suffix != "d" {
                    connection
                        .execute(
                            "INSERT INTO dependency_edges
                             (tenant_id,edge_id,from_type,from_id,relation,to_type,to_id,body_json)
                             VALUES (?1,?2,'claim',?3,'governed_by','memory','schema',?4)",
                            params!["tenant", format!("edge_{suffix}"), claim.claim_id, "{}"],
                        )
                        .unwrap();
                }
            }
            connection
                .execute(
                    "INSERT INTO dependency_edges
                     (tenant_id,edge_id,from_type,from_id,relation,to_type,to_id,body_json)
                     VALUES ('tenant','edge_d_transitive','claim','claim_d',
                             'derived_from','claim','claim_c','{}')",
                    [],
                )
                .unwrap();
        }

        let affected = store
            .invalidate_claims_page(
                "tenant",
                "memory",
                "schema",
                "schema_changed",
                "source/schema/v2",
                2,
            )
            .unwrap();
        assert_eq!(affected, vec!["claim_a", "claim_b"]);
        assert_eq!(
            store
                .get_claim("tenant", "claim_c")
                .unwrap()
                .unwrap()
                .semantic_validity,
            crate::domain::SemanticValidity::Current
        );
        let jobs = store.list_jobs("tenant", 20).unwrap();
        assert_eq!(
            jobs.iter()
                .filter(|job| job.job_type == "claim.revalidate")
                .count(),
            2
        );
        assert_eq!(
            jobs.iter()
                .filter(|job| job.job_type == "invalidation.continue")
                .count(),
            1
        );
        assert_eq!(
            store
                .invalidate_claims_page(
                    "tenant",
                    "memory",
                    "schema",
                    "schema_changed",
                    "source/schema/v2",
                    2,
                )
                .unwrap(),
            affected
        );
        let continuation = store
            .invalidate_claims_page_after(
                "tenant",
                "memory",
                "schema",
                "schema_changed",
                "source/schema/v2/continue/claim_b",
                "source/schema/v2",
                Some("claim_b"),
                2,
                100,
            )
            .unwrap();
        assert_eq!(continuation, vec!["claim_c", "claim_d"]);
        assert_eq!(
            store
                .invalidate_claims_page_after(
                    "tenant",
                    "memory",
                    "schema",
                    "schema_changed",
                    "source/schema/v2/continue/claim_b",
                    "source/schema/v2",
                    Some("claim_b"),
                    2,
                    100,
                )
                .unwrap(),
            continuation
        );
        assert_eq!(
            store
                .list_jobs("tenant", 20)
                .unwrap()
                .iter()
                .filter(|job| job.job_type == "claim.revalidate")
                .count(),
            4
        );
    }

    #[test]
    fn concurrent_admission_returns_one_resource_and_one_outbox_event() {
        let root = TempDir::new().unwrap();
        let database = root.path().join("control.sqlite");
        let first_store = Store::open(&database).unwrap();
        let second_store = Store::open(&database).unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            let candidate =
                admitted_transaction("atxn_first", "request_first", "shared-key", "request-hash");
            first_barrier.wait();
            first_store.create_transaction(&candidate)
        });
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            let candidate = admitted_transaction(
                "atxn_second",
                "request_second",
                "shared-key",
                "request-hash",
            );
            second_barrier.wait();
            second_store.create_transaction(&candidate)
        });
        barrier.wait();

        let first = first.join().unwrap().unwrap();
        let second = second.join().unwrap().unwrap();
        assert_eq!(first.atxn_id, second.atxn_id);

        let inspection = Store::open(&database).unwrap();
        let events = inspection.list_outbox("tenant", 20).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "atxn.admitted")
                .count(),
            1
        );
        assert!(
            inspection
                .get_transaction("tenant", &first.atxn_id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn admission_rejects_a_reused_key_with_a_different_request_hash() {
        let store = Store::in_memory().unwrap();
        store
            .create_transaction(&admitted_transaction(
                "atxn_first",
                "request_first",
                "shared-key",
                "request-hash",
            ))
            .unwrap();

        let result = store.create_transaction(&admitted_transaction(
            "atxn_second",
            "request_second",
            "shared-key",
            "different-hash",
        ));
        assert!(matches!(
            result,
            Err(AmosError::IdempotencyConflict(key)) if key == "shared-key"
        ));
        assert_eq!(store.list_outbox("tenant", 20).unwrap().len(), 1);
    }

    #[test]
    fn concurrent_transitions_use_compare_and_swap_and_emit_one_transition_event() {
        let root = TempDir::new().unwrap();
        let database = root.path().join("control.sqlite");
        Store::open(&database)
            .unwrap()
            .create_transaction(&admitted_transaction(
                "atxn",
                "request",
                "transition-key",
                "request-hash",
            ))
            .unwrap();
        let first_store = Store::open(&database).unwrap();
        let second_store = Store::open(&database).unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_store.transition_transaction(
                "tenant",
                "atxn",
                AtxnState::Admitted,
                0,
                AtxnState::Observing,
                None,
            )
        });
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_store.transition_transaction(
                "tenant",
                "atxn",
                AtxnState::Admitted,
                0,
                AtxnState::Observing,
                None,
            )
        });
        barrier.wait();

        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AmosError::Conflict(_))))
                .count(),
            1
        );

        let inspection = Store::open(&database).unwrap();
        let events = inspection.list_outbox("tenant", 20).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "atxn.transitioned")
                .count(),
            1
        );
        let transaction = inspection
            .get_transaction("tenant", "atxn")
            .unwrap()
            .unwrap();
        assert_eq!(transaction.state, AtxnState::Observing);
        assert_eq!(transaction.state_seq, 1);
    }

    #[test]
    fn execution_commit_is_fenced_and_idempotent_per_plan_step() {
        let store = Store::in_memory().unwrap();
        store
            .create_transaction(&admitted_transaction(
                "atxn",
                "request",
                "execution-key",
                "request-hash",
            ))
            .unwrap();
        let mut transaction = store
            .transition_transaction(
                "tenant",
                "atxn",
                AtxnState::Admitted,
                0,
                AtxnState::Observing,
                None,
            )
            .unwrap();
        for next in [
            AtxnState::Selecting,
            AtxnState::Planning,
            AtxnState::Executing,
        ] {
            transaction = store
                .transition_transaction(
                    "tenant",
                    "atxn",
                    transaction.state,
                    transaction.state_seq,
                    next,
                    None,
                )
                .unwrap();
        }
        let execution = ExecutionRecord {
            execution_id: "execution-first".into(),
            tenant_id: "tenant".into(),
            atxn_id: "atxn".into(),
            plan_id: "plan".into(),
            step_id: "step".into(),
            tool: "sql.readonly.v1".into(),
            tool_version: "1".into(),
            capability_id: "capability-first".into(),
            parameters: json!({"sql":"SELECT 1"}),
            parameters_hash: "parameters-hash".into(),
            input_versions: BTreeMap::from([("warehouse".into(), "v1".into())]),
            output: json!({"value":1}),
            output_hash: "output-hash".into(),
            row_count: 1,
            byte_count: 11,
            latency_ms: 2,
            fencing_token: transaction.state_seq,
            status: "complete".into(),
            created_at: Utc::now(),
        };
        let persisted = store.save_execution(&execution).unwrap();
        let mut duplicate = execution.clone();
        duplicate.execution_id = "execution-duplicate".into();
        duplicate.capability_id = "capability-duplicate".into();
        duplicate.latency_ms = 3;
        let duplicate = store.save_execution(&duplicate).unwrap();
        assert_eq!(duplicate.execution_id, persisted.execution_id);

        let mut conflicting = execution.clone();
        conflicting.execution_id = "execution-conflict".into();
        conflicting.output = json!({"value":2});
        conflicting.output_hash = "different-output-hash".into();
        assert!(matches!(
            store.save_execution(&conflicting),
            Err(AmosError::IdempotencyConflict(_))
        ));
        assert_eq!(
            store
                .list_outbox("tenant", 30)
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "execution.completed")
                .count(),
            1
        );

        store
            .transition_transaction(
                "tenant",
                "atxn",
                AtxnState::Executing,
                transaction.state_seq,
                AtxnState::Composing,
                None,
            )
            .unwrap();
        let mut late = execution;
        late.execution_id = "execution-late".into();
        late.step_id = "late-step".into();
        assert!(matches!(
            store.save_execution(&late),
            Err(AmosError::Conflict(_))
        ));
    }

    #[test]
    fn checkpoints_preserve_fixed_fields_source_versions_and_terminal_immutability() {
        let store = Store::in_memory().unwrap();
        let admitted = store
            .create_transaction(&admitted_transaction(
                "atxn",
                "request",
                "checkpoint-key",
                "request-hash",
            ))
            .unwrap();

        let mut checkpoint = admitted.clone();
        checkpoint
            .source_versions
            .insert("warehouse:payments".into(), "schema-v1".into());
        checkpoint.updated_at += Duration::seconds(1);
        store.checkpoint_transaction(&checkpoint).unwrap();

        let mut rewritten_source = checkpoint.clone();
        rewritten_source
            .source_versions
            .insert("warehouse:payments".into(), "schema-v2".into());
        rewritten_source.updated_at += Duration::seconds(1);
        assert!(matches!(
            store.checkpoint_transaction(&rewritten_source),
            Err(AmosError::Conflict(_))
        ));

        let mut changed_subject = checkpoint.clone();
        changed_subject.subject_id = "different-subject".into();
        changed_subject.updated_at += Duration::seconds(1);
        assert!(matches!(
            store.checkpoint_transaction(&changed_subject),
            Err(AmosError::Conflict(_))
        ));

        let rejected = store
            .transition_transaction(
                "tenant",
                "atxn",
                AtxnState::Admitted,
                0,
                AtxnState::Rejected,
                Some(Outcome::Reject),
            )
            .unwrap();
        let mut terminal_checkpoint = rejected;
        terminal_checkpoint.warnings.push("must not persist".into());
        terminal_checkpoint.updated_at += Duration::seconds(1);
        assert!(matches!(
            store.checkpoint_transaction(&terminal_checkpoint),
            Err(AmosError::Conflict(_))
        ));
    }

    #[test]
    fn expired_final_job_attempt_moves_to_dead_letter_instead_of_redelivery() {
        let store = Store::in_memory().unwrap();
        let start = Utc::now();
        store
            .enqueue_job(&Job {
                job_id: "job".into(),
                tenant_id: "tenant".into(),
                job_type: "test".into(),
                payload: json!({}),
                idempotency_key: "job-key".into(),
                state: JobState::Ready,
                attempt: 0,
                max_attempts: 1,
                fencing_token: 0,
                lease_owner: None,
                lease_expires_at: None,
                next_run_at: start,
                dead_letter_reason: None,
            })
            .unwrap();
        store
            .acquire_job("tenant", "worker-1", start, start + Duration::seconds(10))
            .unwrap()
            .unwrap();

        assert!(
            store
                .acquire_job(
                    "tenant",
                    "worker-2",
                    start + Duration::seconds(11),
                    start + Duration::seconds(30),
                )
                .unwrap()
                .is_none()
        );
        let jobs = store.list_jobs("tenant", 10).unwrap();
        assert_eq!(jobs[0].state, JobState::DeadLetter);
        assert_eq!(
            jobs[0].dead_letter_reason.as_deref(),
            Some("lease expired after final attempt")
        );
        assert!(
            store
                .list_outbox("tenant", 20)
                .unwrap()
                .iter()
                .any(|event| event.event_type == "job.dead_lettered")
        );
    }

    #[test]
    fn outbox_delivery_uses_leases_fences_and_terminal_completion() {
        let store = Store::in_memory().unwrap();
        store
            .enqueue_job(&Job::ready(
                "tenant",
                "test",
                json!({"value":1}),
                "outbox-fixture",
                1,
            ))
            .unwrap();
        let now = Utc::now();
        let first = store
            .acquire_outbox(
                "tenant",
                "dispatcher-a",
                now,
                now + chrono::Duration::seconds(1),
            )
            .unwrap()
            .unwrap();
        assert_eq!(first.state, OutboxState::Running);
        assert_eq!(first.attempt, 1);
        assert!(
            store
                .finish_outbox(&first, first.fencing_token, "dispatcher-b", now, None,)
                .is_err()
        );

        let reacquired = store
            .acquire_outbox(
                "tenant",
                "dispatcher-b",
                now + chrono::Duration::seconds(2),
                now + chrono::Duration::seconds(32),
            )
            .unwrap()
            .unwrap();
        assert!(reacquired.fencing_token > first.fencing_token);
        assert!(reacquired.attempt > first.attempt);
        assert!(
            store
                .finish_outbox(
                    &first,
                    first.fencing_token,
                    "dispatcher-a",
                    now + chrono::Duration::seconds(2),
                    None,
                )
                .is_err()
        );
        let delivered = store
            .finish_outbox(
                &reacquired,
                reacquired.fencing_token,
                "dispatcher-b",
                now + chrono::Duration::seconds(3),
                None,
            )
            .unwrap();
        assert_eq!(delivered.state, OutboxState::Delivered);
        assert!(delivered.completed_at.is_some());
        assert!(
            store
                .acquire_outbox(
                    "tenant",
                    "dispatcher-c",
                    now + chrono::Duration::seconds(4),
                    now + chrono::Duration::seconds(34),
                )
                .unwrap()
                .is_none()
        );
    }
}
