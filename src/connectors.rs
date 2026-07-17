use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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
    events: Arc<Mutex<VecDeque<SourceEvent>>>,
    page_size: usize,
}

impl SqliteWarehouseConnector {
    pub fn new(
        tenant_id: impl Into<String>,
        source_id: impl Into<String>,
        path: impl AsRef<Path>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            source_id: source_id.into(),
            path: path.as_ref().to_path_buf(),
            permissions: BTreeSet::from(["analytics".into(), "payments".into()]),
            events: Arc::new(Mutex::new(VecDeque::new())),
            page_size: 50,
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
        self.events
            .lock()
            .map_err(|_| AmosError::Connector("event queue lock poisoned".into()))?
            .push_back(event.clone());
        Ok(event)
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
        let end = (start + self.page_size).min(names.len());
        let page_names = &names[start..end];
        let next_cursor = (end < names.len()).then(|| names[end - 1].clone());
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
            source_version: content_hash(&snapshot),
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
        if capability.claims.source_id != self.source_id
            || capability.claims.tenant_id != self.tenant_id
        {
            return Err(AmosError::Capability(
                "connector capability scope mismatch".into(),
            ));
        }
        let content = self.schema_snapshot(reference)?;
        let current = content_hash(&content);
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
        match self.schema_snapshot(reference) {
            Ok(snapshot) => {
                let current = content_hash(&snapshot);
                let same = current == observed_version;
                Ok(Validation {
                    same,
                    current_version: Some(current),
                    reason: (!same).then(|| "source version diverged".into()),
                })
            }
            Err(AmosError::NotFound(reason)) => Ok(Validation {
                same: false,
                current_version: None,
                reason: Some(reason),
            }),
            Err(AmosError::Connector(reason)) => Err(AmosError::Connector(reason)),
            Err(other) => Err(other),
        }
    }

    async fn subscribe(&self, cursor: Option<&str>) -> Result<Page<SourceEvent>> {
        let events = self
            .events
            .lock()
            .map_err(|_| AmosError::Connector("event queue lock poisoned".into()))?;
        let items: Vec<SourceEvent> = if let Some(cursor) = cursor {
            if !events.iter().any(|event| event.cursor == cursor) {
                return Err(AmosError::Validation(format!(
                    "subscribe cursor '{cursor}' is unknown"
                )));
            }
            events
                .iter()
                .skip_while(|event| event.cursor != cursor)
                .skip(1)
                .cloned()
                .collect()
        } else {
            events.iter().cloned().collect()
        };
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
    use crate::domain::{CapabilityClaims, CapabilityLimits};
    use tempfile::{NamedTempFile, TempDir};

    fn seed_tables(path: &Path, names: &[&str]) {
        let connection = Connection::open(path).unwrap();
        for name in names {
            connection
                .execute(&format!("CREATE TABLE {name}(id TEXT)"), [])
                .unwrap();
        }
    }

    fn capability(tenant: &str, source: &str) -> CapabilityEnvelope {
        CapabilityEnvelope {
            signature: "test".into(),
            claims: CapabilityClaims {
                issuer: "amos-runtime".into(),
                audience: "sql-worker".into(),
                tenant_id: tenant.into(),
                atxn_id: "atxn".into(),
                plan_id: "plan".into(),
                step_id: "step".into(),
                subject_id: "user".into(),
                tool: "sql.readonly.v1".into(),
                source_id: source.into(),
                operations: BTreeSet::from(["query".into()]),
                relations: BTreeSet::new(),
                limits: CapabilityLimits {
                    seconds: 30,
                    rows: 100,
                    bytes: 1_000,
                },
                policy_epoch: 1,
                fencing_token: 1,
                token_id: "cap".into(),
                not_before: 0,
                expires_at: i64::MAX,
            },
        }
    }

    #[tokio::test]
    async fn connector_has_stable_version_and_detects_change() {
        let file = NamedTempFile::new().unwrap();
        seed_tables(file.path(), &["payments"]);
        let connector = SqliteWarehouseConnector::new("t", "warehouse", file.path());
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
    async fn connector_detects_deletion_and_source_unavailable() {
        let file = NamedTempFile::new().unwrap();
        seed_tables(file.path(), &["payments"]);
        let connector = SqliteWarehouseConnector::new("t", "warehouse", file.path());
        let observed = connector.observe("table:payments").await.unwrap();
        Connection::open(file.path())
            .unwrap()
            .execute("DROP TABLE payments", [])
            .unwrap();
        let validation = connector
            .validate("table:payments", &observed.source_version)
            .await
            .unwrap();
        assert!(!validation.same);
        assert!(validation.current_version.is_none());
        assert!(
            validation
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("deleted"))
        );

        let missing = TempDir::new().unwrap().path().join("missing.sqlite");
        let unavailable = SqliteWarehouseConnector::new("t", "warehouse", &missing);
        let error = unavailable.health().await.unwrap_err();
        assert!(error.to_string().contains("unavailable"));
    }

    #[tokio::test]
    async fn discover_paginates_without_silent_duplicates() {
        let file = NamedTempFile::new().unwrap();
        seed_tables(file.path(), &["alpha", "beta", "gamma", "delta"]);
        let connector =
            SqliteWarehouseConnector::new("t", "warehouse", file.path()).with_page_size(2);
        let first = connector.discover("*", None).await.unwrap();
        assert_eq!(first.items.len(), 2);
        let second = connector
            .discover("*", first.next_cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(second.items.len(), 2);
        let mut refs: Vec<_> = first
            .items
            .iter()
            .chain(second.items.iter())
            .map(|item| item.reference.clone())
            .collect();
        let before = refs.len();
        refs.sort();
        refs.dedup();
        assert_eq!(refs.len(), before);
        assert!(second.next_cursor.is_none() || !second.items.is_empty());
        let err = connector.discover("*", Some("missing-cursor")).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn subscribe_recovers_from_cursor_and_read_is_version_bound() {
        let file = NamedTempFile::new().unwrap();
        seed_tables(file.path(), &["payments"]);
        let connector = SqliteWarehouseConnector::new("t", "warehouse", file.path());
        let first_event = connector
            .emit_change(
                "table:payments",
                Some("v1".into()),
                Some("v2".into()),
                "schema",
            )
            .unwrap();
        let second_event = connector
            .emit_change(
                "table:payments",
                Some("v2".into()),
                Some("v3".into()),
                "schema",
            )
            .unwrap();
        let page = connector
            .subscribe(Some(&first_event.cursor))
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].event_id, second_event.event_id);
        assert!(connector.subscribe(Some("unknown-cursor")).await.is_err());

        let observed = connector.observe("table:payments").await.unwrap();
        let handle = connector
            .read(
                "table:payments",
                &observed.source_version,
                &capability("t", "warehouse"),
            )
            .await
            .unwrap();
        assert_eq!(handle.source_version, observed.source_version);
        assert!(
            connector
                .read("table:payments", "stale", &capability("t", "warehouse"))
                .await
                .is_err()
        );
    }
}
