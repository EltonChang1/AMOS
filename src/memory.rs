use std::{
    cmp::{Ordering, Reverse},
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    domain::{
        Authority, ContextConflict, Identity, MemoryObject, MemoryType, content_hash, new_id,
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
        if object.content_hash != content_hash(&object.content)? {
            return Err(AmosError::Validation("memory content hash mismatch".into()));
        }
        self.store.write_memory(object)
    }

    pub fn list_visible(&self, identity: &Identity) -> Result<Vec<MemoryObject>> {
        Ok(self
            .store
            .list_active_memory(&identity.tenant_id)?
            .into_iter()
            .filter(|object| self.policy.can_read_memory(identity, object))
            .collect())
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
        let fts_query = (!terms.is_empty()).then(|| {
            terms
                .iter()
                .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ")
        });
        let candidate_limit = query.max_items.max(1).saturating_mul(8).min(2_000);
        let candidates = self.store.retrieve_memory_candidates(
            &identity.tenant_id,
            &identity.permissions,
            &query.required_types,
            query.time_start,
            query.time_end,
            fts_query.as_deref(),
            candidate_limit,
        )?;
        let max_items = query.max_items.max(1);
        let mut top = BinaryHeap::with_capacity(max_items + 1);
        for object in candidates
            .into_iter()
            .filter(|object| self.policy.can_read_memory(identity, object))
        {
            let score = retrieval_score(&object, &terms);
            if score <= 0 && !terms.is_empty() {
                continue;
            }
            top.push(Reverse(RankedMemory::new(score, object)));
            if top.len() > max_items {
                top.pop();
            }
        }
        let mut visible = top
            .into_iter()
            .map(|Reverse(item)| item)
            .collect::<Vec<_>>();
        visible.sort_by(|left, right| right.cmp(left));
        Ok(RetrievalResult {
            items: visible.into_iter().map(|item| item.object).collect(),
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
            let Some(highest) = candidates.first().map(|item| item.authority.rank()) else {
                continue;
            };
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
        )?;
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
        compacted.content_hash = content_hash(&compacted.content)?;
        Ok(compacted)
    }
}

struct RankedMemory {
    score: i64,
    authority: u8,
    recorded_at: i64,
    object_id: String,
    object: MemoryObject,
}

impl RankedMemory {
    fn new(score: i64, object: MemoryObject) -> Self {
        Self {
            score,
            authority: object.authority.rank(),
            recorded_at: object.recorded_at.timestamp_micros(),
            object_id: object.object_id.clone(),
            object,
        }
    }

    fn key(&self) -> (i64, u8, i64, Reverse<&str>) {
        (
            self.score,
            self.authority,
            self.recorded_at,
            Reverse(self.object_id.as_str()),
        )
    }
}

impl PartialEq for RankedMemory {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for RankedMemory {}

impl PartialOrd for RankedMemory {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankedMemory {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key().cmp(&other.key())
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
        format!("{:?}", self.memory_type)
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
        )
        .unwrap();
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

    #[test]
    fn retrieval_pushes_scope_filters_into_bounded_fts_candidates() {
        let store = Store::in_memory().unwrap();
        let service = MemoryService::new(store.clone(), PolicyEngine);
        for index in 0_u64..500 {
            let mut object = MemoryObject::new(
                "t",
                format!("document:{index}"),
                MemoryType::Document,
                format!("needle incident document {index}"),
                json!({"body":format!("needle evidence {index}")}),
                "documents",
                index.to_string(),
                Authority::SystemObserved,
            )
            .unwrap();
            object.permissions = if index.is_multiple_of(2) {
                BTreeSet::from(["payments".into()])
            } else {
                BTreeSet::from(["restricted".into()])
            };
            store.write_memory(&object).unwrap();
        }
        let result = service
            .retrieve(
                &identity(&["payments"]),
                &RetrieveQuery {
                    task_text: "needle incident".into(),
                    required_types: BTreeSet::from([MemoryType::Document]),
                    time_start: Utc::now() - Duration::days(1),
                    time_end: Utc::now(),
                    max_items: 7,
                },
            )
            .unwrap();
        assert_eq!(result.items.len(), 7);
        assert!(result.items.iter().all(|object| {
            object.memory_type == MemoryType::Document
                && object.permissions == BTreeSet::from(["payments".into()])
        }));
    }
}
