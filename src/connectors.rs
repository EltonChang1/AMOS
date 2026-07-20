use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{
        CapabilityEnvelope, ConnectorHealth, ConsistencyClass, SourceEvent, SourceObservation,
        content_hash, new_id,
    },
    error::AmosError,
    workers::CapabilityIssuer,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntityRef {
    pub source_id: String,
    pub reference: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validation {
    pub same: bool,
    pub current_version: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoundedHandle {
    pub source_id: String,
    pub reference: String,
    pub source_version: String,
    pub content: Value,
    pub content_hash: String,
}

#[async_trait]
pub trait Connector: Send + Sync {
    fn source_id(&self) -> &str;
    async fn discover(&self, scope: &str, cursor: Option<&str>) -> Result<Page<EntityRef>>;
    async fn observe(&self, reference: &str) -> Result<SourceObservation>;
    async fn read(
        &self,
        reference: &str,
        source_version: &str,
        capability: &CapabilityEnvelope,
    ) -> Result<BoundedHandle>;
    async fn validate(&self, reference: &str, observed_version: &str) -> Result<Validation>;
    async fn subscribe(&self, cursor: Option<&str>) -> Result<Page<SourceEvent>>;
    async fn health(&self) -> Result<ConnectorHealth>;
}

#[derive(Clone)]
pub struct SqliteWarehouseConnector {
    tenant_id: String,
    source_id: String,
    path: PathBuf,
    permissions: BTreeSet<String>,
    capability_issuer: CapabilityIssuer,
}

impl SqliteWarehouseConnector {
    pub fn new(
        tenant_id: impl Into<String>,
        source_id: impl Into<String>,
        path: impl AsRef<Path>,
        capability_issuer: CapabilityIssuer,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            source_id: source_id.into(),
            path: path.as_ref().to_path_buf(),
            permissions: BTreeSet::from(["analytics".into(), "payments".into()]),
            capability_issuer,
        }
    }

    pub fn with_page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn emit_change(
        &self,
        subject: &str,
        previous: Option<String>,
        current: Option<String>,
        kind: &str,
    ) -> Result<SourceEvent> {
        let event = SourceEvent {
            event_id: new_id("evt_src"),
            tenant_id: self.tenant_id.clone(),
            source_id: self.source_id.clone(),
            subject: subject.into(),
            previous_version: previous,
            current_version: current.clone(),
            change_kind: kind.into(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            cursor: new_id("cur"),
            deduplication_key: format!(
                "{}/{}/{}",
                self.source_id,
                subject,
                current.unwrap_or_else(|| "deleted".into())
            ),
        };
        let connection = self.event_connection()?;
        let changed = connection.execute(
            "INSERT OR IGNORE INTO amos_connector_events
             (tenant_id,event_id,deduplication_key,cursor,observed_at,body_json)
             VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                event.tenant_id,
                event.event_id,
                event.deduplication_key,
                event.cursor,
                event.observed_at.to_rfc3339(),
                serde_json::to_string(&event)?,
            ],
        )?;
        if changed == 1 {
            return Ok(event);
        }
        let body: String = connection.query_row(
            "SELECT body_json FROM amos_connector_events
             WHERE tenant_id=?1 AND deduplication_key=?2",
            rusqlite::params![self.tenant_id, event.deduplication_key],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&body)?)
    }

    fn event_connection(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)
            .map_err(|error| AmosError::Connector(error.to_string()))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS amos_connector_events(
                tenant_id TEXT NOT NULL,
                event_id TEXT NOT NULL,
                deduplication_key TEXT NOT NULL,
                cursor TEXT NOT NULL,
                observed_at TEXT NOT NULL,
                body_json TEXT NOT NULL,
                PRIMARY KEY(tenant_id,event_id),
                UNIQUE(tenant_id,deduplication_key),
                UNIQUE(tenant_id,cursor)
             );",
        )?;
        Ok(connection)
    }

    fn open_connection(&self) -> Result<Connection> {
        if !self.path.exists() {
            return Err(AmosError::Connector(
                "warehouse source unavailable: database file missing".into(),
            ));
        }
        Connection::open(&self.path)
            .map_err(|error| AmosError::Connector(format!("warehouse source unavailable: {error}")))
    }

    fn schema_snapshot(&self, reference: &str) -> Result<Value> {
        let connection = self.open_connection()?;
        let table = reference.strip_prefix("table:").unwrap_or(reference);
        let exists: bool = connection
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .map_err(|error| AmosError::Connector(error.to_string()))?;
        if !exists {
            return Err(AmosError::NotFound(format!(
                "table {table} deleted or unavailable"
            )));
        }
        let mut statement = connection.prepare(&format!(
            "PRAGMA table_info('{}')",
            table.replace('\'', "''")
        ))?;
        let columns = statement
            .query_map([], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(1)?,
                    "type": row.get::<_, String>(2)?,
                    "not_null": row.get::<_, bool>(3)?
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if columns.is_empty() {
            return Err(AmosError::NotFound(format!("table {table}")));
        }
        Ok(json!({"table": table, "columns": columns}))
    }

    fn list_tables(&self, scope: &str) -> Result<Vec<String>> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(names
            .into_iter()
            .filter(|name| scope == "*" || name.contains(scope))
            .collect())
    }
}

#[async_trait]
impl Connector for SqliteWarehouseConnector {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn discover(&self, scope: &str, cursor: Option<&str>) -> Result<Page<EntityRef>> {
        let names = self.list_tables(scope)?;
        let start = cursor
            .and_then(|value| {
                names
                    .iter()
                    .position(|name| name == value)
                    .map(|idx| idx + 1)
            })
            .unwrap_or(0);
        if cursor.is_some() && start == 0 {
            return Err(AmosError::Validation(format!(
                "discover cursor '{}' is unknown for scope '{scope}'",
                cursor.unwrap_or_default()
            )));
        }
        let connection = Connection::open(&self.path)
            .map_err(|error| AmosError::Connector(error.to_string()))?;
        let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'amos_%' ORDER BY name")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Page {
            items: page_names
                .iter()
                .map(|name| EntityRef {
                    source_id: self.source_id.clone(),
                    reference: format!("table:{name}"),
                    entity_type: "relation".into(),
                })
                .collect(),
            next_cursor,
        })
    }

    async fn observe(&self, reference: &str) -> Result<SourceObservation> {
        let snapshot = self.schema_snapshot(reference)?;
        Ok(SourceObservation {
            tenant_id: self.tenant_id.clone(),
            source_id: self.source_id.clone(),
            reference: reference.into(),
            source_version: content_hash(&snapshot)?,
            observed_at: Utc::now(),
            effective_start: None,
            effective_end: None,
            freshness_seconds: 0,
            sensitivity: "internal".into(),
            permissions: self.permissions.clone(),
            consistency_class: ConsistencyClass::C1,
            retention_deadline: None,
        })
    }

    async fn read(
        &self,
        reference: &str,
        source_version: &str,
        capability: &CapabilityEnvelope,
    ) -> Result<BoundedHandle> {
        self.capability_issuer.validate(
            capability,
            "sql-worker",
            capability.claims.policy_epoch,
            capability.claims.fencing_token,
        )?;
        let relation = reference.strip_prefix("table:").unwrap_or(reference);
        if capability.claims.source_id != self.source_id
            || capability.claims.tenant_id != self.tenant_id
            || capability.claims.tool != "sql.readonly.v1"
            || capability.claims.operations != BTreeSet::from(["query".to_string()])
            || (!capability.claims.relations.contains(reference)
                && !capability.claims.relations.contains(relation))
            || capability.claims.limits.seconds == 0
            || capability.claims.limits.rows == 0
            || capability.claims.limits.bytes == 0
            || capability.claims.atxn_id.trim().is_empty()
            || capability.claims.plan_id.trim().is_empty()
            || capability.claims.step_id.trim().is_empty()
            || capability.claims.subject_id.trim().is_empty()
        {
            return Err(AmosError::Capability(
                "connector capability is not fully bound to the requested source read".into(),
            ));
        }
        let content = self.schema_snapshot(reference)?;
        let current = content_hash(&content)?;
        if current != source_version {
            return Err(AmosError::Conflict("source version changed".into()));
        }
        Ok(BoundedHandle {
            source_id: self.source_id.clone(),
            reference: reference.into(),
            source_version: current.clone(),
            content,
            content_hash: current,
        })
    }

    async fn validate(&self, reference: &str, observed_version: &str) -> Result<Validation> {
        let current = content_hash(&self.schema_snapshot(reference)?)?;
        Ok(Validation {
            same: current == observed_version,
            current_version: Some(current),
            reason: None,
        })
    }

    async fn subscribe(&self, cursor: Option<&str>) -> Result<Page<SourceEvent>> {
        let connection = self.event_connection()?;
        let after_rowid = if let Some(cursor) = cursor {
            Some(
                connection
                    .query_row(
                        "SELECT rowid FROM amos_connector_events
                         WHERE tenant_id=?1 AND cursor=?2",
                        rusqlite::params![self.tenant_id, cursor],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|_| {
                        AmosError::Validation("connector cursor is unknown or expired".into())
                    })?,
            )
        } else {
            None
        };
        let mut statement = connection.prepare(
            "SELECT body_json FROM amos_connector_events
             WHERE tenant_id=?1 AND rowid>?2 ORDER BY rowid LIMIT 250",
        )?;
        let bodies = statement
            .query_map(
                rusqlite::params![self.tenant_id, after_rowid.unwrap_or(0)],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let items = bodies
            .into_iter()
            .map(|body| serde_json::from_str(&body).map_err(Into::into))
            .collect::<Result<Vec<SourceEvent>>>()?;
        let next_cursor = items.last().map(|event| event.cursor.clone());
        Ok(Page { items, next_cursor })
    }

    async fn health(&self) -> Result<ConnectorHealth> {
        self.open_connection()?;
        Ok(ConnectorHealth {
            source_id: self.source_id.clone(),
            status: "healthy".into(),
            lag_seconds: 0,
            rate_limit_remaining: None,
            degraded_capabilities: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Identity, OperationLimits, PlanStep, TypedPlan};
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn connector_has_stable_version_and_detects_change() {
        let file = NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection
            .execute("CREATE TABLE payments(id TEXT)", [])
            .unwrap();
        drop(connection);
        let connector = SqliteWarehouseConnector::new(
            "t",
            "warehouse",
            file.path(),
            CapabilityIssuer::new([9_u8; 32]).unwrap(),
        );
        let first = connector.observe("table:payments").await.unwrap();
        let second = connector.observe("table:payments").await.unwrap();
        assert_eq!(first.source_version, second.source_version);
        Connection::open(file.path())
            .unwrap()
            .execute("ALTER TABLE payments ADD COLUMN amount REAL", [])
            .unwrap();
        assert!(
            !connector
                .validate("table:payments", &first.source_version)
                .await
                .unwrap()
                .same
        );
    }

    #[tokio::test]
    async fn connector_verifies_signature_and_binds_the_requested_relation() {
        let file = NamedTempFile::new().unwrap();
        Connection::open(file.path())
            .unwrap()
            .execute("CREATE TABLE payments(id TEXT)", [])
            .unwrap();
        let issuer = CapabilityIssuer::new([4_u8; 32]).unwrap();
        let connector =
            SqliteWarehouseConnector::new("t", "warehouse", file.path(), issuer.clone());
        let identity = Identity {
            tenant_id: "t".into(),
            subject_id: "u".into(),
            roles: BTreeSet::new(),
            groups: BTreeSet::new(),
            permissions: BTreeSet::from(["payments".into()]),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 3,
        };
        let mut step = PlanStep {
            step_id: "s".into(),
            purpose: "read schema".into(),
            tool: "sql.readonly.v1".into(),
            source_id: "warehouse".into(),
            input_object_ids: vec![],
            parameter_schema: "schema.v1".into(),
            parameters: json!({"relations":["payments"]}),
            expected_output_schema: "schema.v1".into(),
            limits: OperationLimits {
                seconds: 1,
                rows: 10,
                bytes: 1_000,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: "test.v1".into(),
        };
        let mut plan = TypedPlan {
            plan_id: "p".into(),
            tenant_id: "t".into(),
            atxn_id: "a".into(),
            task_definition: "task:v1".into(),
            manifest_id: "m".into(),
            model_identity: "deterministic".into(),
            steps: vec![step.clone()],
        };
        let observation = connector.observe("table:payments").await.unwrap();
        let capability = issuer.issue(&identity, &plan, &step, 1).unwrap();
        connector
            .read("table:payments", &observation.source_version, &capability)
            .await
            .unwrap();

        step.parameters = json!({"relations":["other"]});
        plan.steps = vec![step.clone()];
        let wrong_relation = issuer.issue(&identity, &plan, &step, 1).unwrap();
        assert!(matches!(
            connector
                .read(
                    "table:payments",
                    &observation.source_version,
                    &wrong_relation,
                )
                .await,
            Err(AmosError::Capability(_))
        ));
    }

    #[tokio::test]
    async fn connector_events_are_durable_deduplicated_and_cursor_bounded() {
        let file = NamedTempFile::new().unwrap();
        let issuer = CapabilityIssuer::new([7_u8; 32]).unwrap();
        let connector =
            SqliteWarehouseConnector::new("t", "warehouse", file.path(), issuer.clone());
        let first = connector
            .emit_change(
                "table:payments",
                Some("v1".into()),
                Some("v2".into()),
                "schema",
            )
            .unwrap();
        let duplicate = connector
            .emit_change(
                "table:payments",
                Some("v1".into()),
                Some("v2".into()),
                "schema",
            )
            .unwrap();
        assert_eq!(duplicate.event_id, first.event_id);

        drop(connector);
        let reopened = SqliteWarehouseConnector::new("t", "warehouse", file.path(), issuer);
        let page = reopened.subscribe(None).await.unwrap();
        assert_eq!(page.items, vec![first.clone()]);
        assert_eq!(page.next_cursor.as_deref(), Some(first.cursor.as_str()));
        assert!(
            reopened
                .subscribe(page.next_cursor.as_deref())
                .await
                .unwrap()
                .items
                .is_empty()
        );
        assert!(matches!(
            reopened.subscribe(Some("unknown")).await,
            Err(AmosError::Validation(_))
        ));
    }
}
