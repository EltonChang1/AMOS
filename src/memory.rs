use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    domain::{
        Authority, ContextConflict, Identity, MemoryObject, MemoryStatus, MemoryType, content_hash,
        new_id,
    },
    error::AmosError,
    policy::PolicyEngine,
    store::Store,
};

#[derive(Debug, Clone)]
pub struct RetrieveQuery {
    pub task_text: String,
    pub required_types: BTreeSet<MemoryType>,
    pub time_start: DateTime<Utc>,
    pub time_end: DateTime<Utc>,
    pub max_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub items: Vec<MemoryObject>,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
pub struct MemoryService {
    store: Store,
    policy: PolicyEngine,
}

impl MemoryService {
    pub fn new(store: Store, policy: PolicyEngine) -> Self {
        Self { store, policy }
    }

    pub fn write(&self, identity: &Identity, object: &MemoryObject) -> Result<()> {
        self.policy.authorize_memory_write(identity, object)?;
        if object.logical_key.trim().is_empty() || object.source_id.trim().is_empty() {
            return Err(AmosError::Validation(
                "memory requires logical key and source".into(),
            ));
        }
        if object.content_hash != content_hash(&object.content) {
            return Err(AmosError::Validation("memory content hash mismatch".into()));
        }
        self.store.write_memory(object)
    }

    pub fn supersede(
        &self,
        identity: &Identity,
        old_id: &str,
        mut new_object: MemoryObject,
    ) -> Result<MemoryObject> {
        let old = self
            .store
            .get_memory(&identity.tenant_id, old_id)?
            .ok_or_else(|| AmosError::NotFound(old_id.into()))?;
        if !self.policy.can_read_memory(identity, &old) {
            return Err(AmosError::PermissionDenied("memory is not visible".into()));
        }
        self.policy.authorize_memory_write(identity, &new_object)?;
        if old.logical_key != new_object.logical_key {
            return Err(AmosError::Validation(
                "supersession requires the same logical key".into(),
            ));
        }
        if !new_object.supersedes.contains(&old.object_id) {
            new_object.supersedes.push(old.object_id.clone());
        }
        self.store
            .supersede_memory(&identity.tenant_id, old_id, &new_object)?;
        Ok(new_object)
    }

    pub fn retrieve(&self, identity: &Identity, query: &RetrieveQuery) -> Result<RetrievalResult> {
        let terms = terms(&query.task_text);
        let mut visible: Vec<(i64, MemoryObject)> = self
            .store
            .list_active_memory(&identity.tenant_id)?
            .into_iter()
            .filter(|object| self.policy.can_read_memory(identity, object))
            .filter(|object| {
                object.status == MemoryStatus::Active && object.superseded_by.is_none()
            })
            .filter(|object| object.effective_at(query.time_start, query.time_end))
            .filter(|object| {
                query.required_types.is_empty()
                    || query.required_types.contains(&object.memory_type)
            })
            .map(|object| (retrieval_score(&object, &terms), object))
            .filter(|(score, _)| *score > 0 || terms.is_empty())
            .collect();
        visible.sort_by_key(|(score, object)| {
            (
                Reverse(*score),
                Reverse(object.authority.rank()),
                Reverse(object.recorded_at),
            )
        });
        visible.truncate(query.max_items.max(1));
        Ok(RetrievalResult {
            items: visible.into_iter().map(|(_, object)| object).collect(),
            warnings: vec![],
        })
    }

    pub fn reconcile(&self, items: Vec<MemoryObject>) -> (Vec<MemoryObject>, Vec<ContextConflict>) {
        let mut groups: BTreeMap<(MemoryType, String), Vec<MemoryObject>> = BTreeMap::new();
        for item in items {
            groups
                .entry((item.memory_type, item.logical_key.clone()))
                .or_default()
                .push(item);
        }
        let mut selected = vec![];
        let mut conflicts = vec![];
        for ((_memory_type, logical_key), mut candidates) in groups {
            candidates
                .sort_by_key(|item| (Reverse(item.authority.rank()), Reverse(item.recorded_at)));
            let highest = candidates
                .first()
                .map(|item| item.authority.rank())
                .unwrap_or_default();
            let peers: Vec<_> = candidates
                .iter()
                .filter(|item| item.authority.rank() == highest)
                .collect();
            let peer_hashes: BTreeSet<_> = peers.iter().map(|item| &item.content_hash).collect();
            if peer_hashes.len() > 1 {
                conflicts.push(ContextConflict {
                    logical_key,
                    object_ids: peers.iter().map(|item| item.object_id.clone()).collect(),
                    reason: "equal-authority active versions disagree".into(),
                });
            } else if let Some(item) = candidates.into_iter().next() {
                selected.push(item);
            }
        }
        (selected, conflicts)
    }

    pub fn compact(
        &self,
        identity: &Identity,
        items: &[MemoryObject],
        summary: String,
    ) -> Result<MemoryObject> {
        if items.is_empty() {
            return Err(AmosError::Validation("cannot compact an empty set".into()));
        }
        if items
            .iter()
            .any(|item| !self.policy.can_read_memory(identity, item))
        {
            return Err(AmosError::PermissionDenied(
                "compaction source is not visible".into(),
            ));
        }
        let mut compacted = MemoryObject::new(
            &identity.tenant_id,
            format!("compaction:{}", new_id("set")),
            MemoryType::ActiveContext,
            summary.clone(),
            serde_json::json!({"summary":summary,"source_object_ids":items.iter().map(|item|&item.object_id).collect::<Vec<_>>() }),
            "amos.compactor",
            new_id("v"),
            Authority::SystemObserved,
        );
        compacted.permissions = items
            .iter()
            .flat_map(|item| item.permissions.iter().cloned())
            .collect();
        compacted.effective_start = items.iter().filter_map(|item| item.effective_start).max();
        compacted.effective_end = items.iter().filter_map(|item| item.effective_end).min();
        compacted.provenance_ref = Some(
            items
                .iter()
                .map(|item| item.object_id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
        compacted.governing = false;
        compacted.content_hash = content_hash(&compacted.content);
        Ok(compacted)
    }
}

fn terms(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() > 2)
        .collect()
}

fn retrieval_score(object: &MemoryObject, terms: &BTreeSet<String>) -> i64 {
    let searchable = format!(
        "{} {} {} {}",
        object.logical_key,
        object.summary,
        object.memory_type_string(),
        object.content
    )
    .to_lowercase();
    let matches = terms
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count() as i64;
    matches * 100 + i64::from(object.authority.rank()) * 5 + if object.governing { 3 } else { 0 }
}

trait MemoryTypeString {
    fn memory_type_string(&self) -> String;
}
impl MemoryTypeString for MemoryObject {
    fn memory_type_string(&self) -> String {
        serde_json::to_string(&self.memory_type).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn identity(permissions: &[&str]) -> Identity {
        Identity {
            tenant_id: "t".into(),
            subject_id: "u".into(),
            roles: BTreeSet::from(["admin".into()]),
            groups: BTreeSet::new(),
            permissions: permissions.iter().map(|v| v.to_string()).collect(),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 1,
        }
    }

    #[test]
    fn retrieve_filters_permissions_before_results() {
        let store = Store::in_memory().unwrap();
        let service = MemoryService::new(store.clone(), PolicyEngine);
        let mut public = MemoryObject::new(
            "t",
            "metric:failure",
            MemoryType::SemanticDefinition,
            "payment failure metric",
            json!({}),
            "semantic",
            "1",
            Authority::OwnerApproved,
        );
        public.permissions.insert("payments".into());
        let mut secret = public.clone();
        secret.object_id = new_id("mem");
        secret.logical_key = "metric:secret".into();
        secret.source_version = "2".into();
        secret.permissions.insert("sre".into());
        store.write_memory(&public).unwrap();
        store.write_memory(&secret).unwrap();
        let result = service
            .retrieve(
                &identity(&["payments"]),
                &RetrieveQuery {
                    task_text: "payment failure".into(),
                    required_types: BTreeSet::new(),
                    time_start: Utc::now() - Duration::days(1),
                    time_end: Utc::now(),
                    max_items: 10,
                },
            )
            .unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].logical_key, "metric:failure");
    }
}
