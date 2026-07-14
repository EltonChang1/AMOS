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
        }
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

    fn schema_snapshot(&self, reference: &str) -> Result<Value> {
        let connection = Connection::open(&self.path)
            .map_err(|error| AmosError::Connector(error.to_string()))?;
        let table = reference.strip_prefix("table:").unwrap_or(reference);
        let mut statement = connection.prepare(&format!(
            "PRAGMA table_info('{}')",
            table.replace('\'', "''")
        ))?;
        let columns = statement.query_map([], |row| Ok(json!({"name":row.get::<_,String>(1)?,"type":row.get::<_,String>(2)?,"not_null":row.get::<_,bool>(3)?})))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        if columns.is_empty() {
            return Err(AmosError::NotFound(format!("table {table}")));
        }
        Ok(json!({"table":table,"columns":columns}))
    }
}

#[async_trait]
impl Connector for SqliteWarehouseConnector {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    async fn discover(&self, scope: &str, cursor: Option<&str>) -> Result<Page<EntityRef>> {
        if cursor.is_some() {
            return Ok(Page {
                items: vec![],
                next_cursor: None,
            });
        }
        let connection = Connection::open(&self.path)
            .map_err(|error| AmosError::Connector(error.to_string()))?;
        let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Page {
            items: names
                .into_iter()
                .filter(|name| scope == "*" || name.contains(scope))
                .map(|name| EntityRef {
                    source_id: self.source_id.clone(),
                    reference: format!("table:{name}"),
                    entity_type: "relation".into(),
                })
                .collect(),
            next_cursor: None,
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
        let current = content_hash(&self.schema_snapshot(reference)?);
        Ok(Validation {
            same: current == observed_version,
            current_version: Some(current),
            reason: None,
        })
    }

    async fn subscribe(&self, cursor: Option<&str>) -> Result<Page<SourceEvent>> {
        let events = self
            .events
            .lock()
            .map_err(|_| AmosError::Connector("event queue lock poisoned".into()))?;
        let items: Vec<SourceEvent> = if let Some(cursor) = cursor {
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
        if !self.path.is_file() {
            return Err(AmosError::Connector("warehouse file is unavailable".into()));
        }
        Connection::open(&self.path).map_err(|error| AmosError::Connector(error.to_string()))?;
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
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn connector_has_stable_version_and_detects_change() {
        let file = NamedTempFile::new().unwrap();
        let connection = Connection::open(file.path()).unwrap();
        connection
            .execute("CREATE TABLE payments(id TEXT)", [])
            .unwrap();
        drop(connection);
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
}
