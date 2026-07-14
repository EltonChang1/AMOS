use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::{
    Result,
    domain::{
        ContextManifest, ContextOmission, Identity, MemoryObject, MemoryType, TaskDefinition,
        new_id,
    },
    error::AmosError,
    memory::{MemoryService, RetrieveQuery},
};

#[derive(Clone)]
pub struct ContextCompiler {
    memory: MemoryService,
}

impl ContextCompiler {
    pub fn new(memory: MemoryService) -> Self {
        Self { memory }
    }

    pub fn compile(
        &self,
        identity: &Identity,
        atxn_id: &str,
        request: &str,
        definition: &TaskDefinition,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<ContextManifest> {
        let types: BTreeSet<MemoryType> = definition
            .required_roles
            .values()
            .chain(definition.optional_roles.values())
            .copied()
            .collect();
        let retrieved = self.memory.retrieve(
            identity,
            &RetrieveQuery {
                task_text: request.into(),
                required_types: types,
                time_start: start,
                time_end: end,
                max_items: definition.budgets.max_context_items.saturating_mul(4),
            },
        )?;
        let (reconciled, conflicts) = self.memory.reconcile(retrieved.items);
        if !conflicts.is_empty() {
            return Ok(ContextManifest {
                manifest_id: new_id("ctx"),
                tenant_id: identity.tenant_id.clone(),
                atxn_id: atxn_id.into(),
                task_definition: format!("{}:v{}", definition.task_type, definition.version),
                policy_epoch: identity.policy_epoch,
                required_role_coverage: BTreeMap::new(),
                optional_selected: vec![],
                omissions: vec![],
                conflicts,
                token_count: 0,
                source_versions: BTreeMap::new(),
                selected_objects: vec![],
                warnings: retrieved.warnings,
            });
        }

        let mut required_coverage = BTreeMap::new();
        let mut selected: Vec<MemoryObject> = vec![];
        let mut omissions = vec![];
        for (role, memory_type) in &definition.required_roles {
            let candidates = role_candidates(&reconciled, role, *memory_type);
            let Some(candidate) = candidates.first() else {
                return Err(AmosError::RequiredRoleMissing(role.clone()));
            };
            if !candidate.governing {
                return Err(AmosError::RequiredRoleMissing(format!(
                    "{role} cannot use compacted memory"
                )));
            }
            required_coverage.insert(role.clone(), vec![candidate.object_id.clone()]);
            if !selected
                .iter()
                .any(|item| item.object_id == candidate.object_id)
            {
                selected.push((*candidate).clone());
            }
        }
        let mut optional_selected = vec![];
        for (role, memory_type) in &definition.optional_roles {
            if let Some(candidate) = role_candidates(&reconciled, role, *memory_type).first() {
                if selected.len() < definition.budgets.max_context_items {
                    optional_selected.push(candidate.object_id.clone());
                    if !selected
                        .iter()
                        .any(|item| item.object_id == candidate.object_id)
                    {
                        selected.push((*candidate).clone());
                    }
                }
            } else {
                omissions.push(ContextOmission {
                    role: role.clone(),
                    reason: "no_authorized_candidate".into(),
                });
            }
        }
        let token_count: usize = selected.iter().map(estimate_tokens).sum();
        if token_count > definition.budgets.max_context_tokens {
            return Err(AmosError::Validation(
                "required context exceeds token budget".into(),
            ));
        }
        let source_versions = selected
            .iter()
            .map(|item| {
                (
                    format!("{}:{}", item.source_id, item.logical_key),
                    item.source_version.clone(),
                )
            })
            .collect();
        Ok(ContextManifest {
            manifest_id: new_id("ctx"),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: atxn_id.into(),
            task_definition: format!("{}:v{}", definition.task_type, definition.version),
            policy_epoch: identity.policy_epoch,
            required_role_coverage: required_coverage,
            optional_selected,
            omissions,
            conflicts: vec![],
            token_count,
            source_versions,
            selected_objects: selected,
            warnings: retrieved.warnings,
        })
    }
}

fn role_candidates<'a>(
    items: &'a [MemoryObject],
    role: &str,
    memory_type: MemoryType,
) -> Vec<&'a MemoryObject> {
    let mut candidates: Vec<_> = items
        .iter()
        .filter(|item| item.memory_type == memory_type)
        .filter(|item| {
            item.content
                .get("role")
                .and_then(|value| value.as_str())
                .is_none_or(|value| value == role)
        })
        .collect();
    candidates.sort_by_key(|item| std::cmp::Reverse(item.authority.rank()));
    candidates
}

fn estimate_tokens(item: &MemoryObject) -> usize {
    (item.summary.len() + item.content.to_string().len()).div_ceil(4)
}
