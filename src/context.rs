use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::{
    Result,
    domain::{
        ConsistencyClass, ContextManifest, ContextOmission, ContextRankingEntry, Identity,
        MemoryObject, MemoryType, TaskDefinition, stable_id,
    },
    error::AmosError,
    memory::{MemoryService, RetrieveQuery},
};

const CONTEXT_TOKENIZER: &str = "amos_lexical_v1";

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
        let now = Utc::now();
        if definition.tenant_id != identity.tenant_id
            || definition.status != "approved"
            || start > end
            || definition.effective_start.is_some_and(|value| value > now)
            || definition.effective_end.is_some_and(|value| value < now)
        {
            return Err(AmosError::Validation(
                "task definition is not approved and effective for this tenant and interval".into(),
            ));
        }
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
                manifest_id: stable_id(
                    "ctx",
                    &(
                        identity.tenant_id.as_str(),
                        atxn_id,
                        definition.task_type.as_str(),
                        definition.version,
                    ),
                )?,
                tenant_id: identity.tenant_id.clone(),
                atxn_id: atxn_id.into(),
                task_definition: format!("{}:v{}", definition.task_type, definition.version),
                policy_epoch: identity.policy_epoch,
                required_role_coverage: BTreeMap::new(),
                optional_selected: vec![],
                omissions: vec![],
                conflicts,
                token_count: 0,
                tokenizer: CONTEXT_TOKENIZER.into(),
                ranking_trace: vec![],
                source_versions: BTreeMap::new(),
                selected_objects: vec![],
                warnings: retrieved.warnings,
            });
        }

        let mut required_coverage = BTreeMap::new();
        let mut selected: Vec<MemoryObject> = vec![];
        let mut omissions = vec![];
        let mut ranking_trace = vec![];
        let mut token_count: usize = 0;
        for (role, memory_type) in &definition.required_roles {
            let candidates = role_candidates(&reconciled, role, *memory_type, request);
            let minimum = definition
                .minimum_consistency
                .get(role)
                .copied()
                .unwrap_or(ConsistencyClass::C0);
            let eligible = candidates
                .iter()
                .copied()
                .filter(|candidate| candidate.consistency_class >= minimum)
                .collect::<Vec<_>>();
            append_ranking_trace(
                &mut ranking_trace,
                role,
                &candidates,
                request,
                eligible.first().copied(),
                minimum,
            );
            ensure_unambiguous(role, &eligible, request)?;
            let Some(candidate) = eligible.first() else {
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
                if selected.len() >= definition.budgets.max_context_items {
                    return Err(AmosError::Validation(
                        "required context exceeds item budget".into(),
                    ));
                }
                token_count = token_count
                    .checked_add(exact_token_count(candidate)?)
                    .ok_or_else(|| AmosError::Validation("context token count overflow".into()))?;
                if token_count > definition.budgets.max_context_tokens {
                    return Err(AmosError::Validation(
                        "required context exceeds token budget".into(),
                    ));
                }
                selected.push((*candidate).clone());
            }
        }
        let mut optional_selected = vec![];
        for (role, memory_type) in &definition.optional_roles {
            let candidates = role_candidates(&reconciled, role, *memory_type, request);
            let minimum = definition
                .minimum_consistency
                .get(role)
                .copied()
                .unwrap_or(ConsistencyClass::C0);
            let eligible = candidates
                .iter()
                .copied()
                .filter(|candidate| candidate.consistency_class >= minimum)
                .collect::<Vec<_>>();
            let ambiguous = is_ambiguous(&eligible, request);
            let candidate = (!ambiguous).then(|| eligible.first().copied()).flatten();
            append_ranking_trace(
                &mut ranking_trace,
                role,
                &candidates,
                request,
                candidate,
                minimum,
            );
            if ambiguous {
                omissions.push(ContextOmission {
                    role: role.clone(),
                    reason: "ambiguous_authorized_candidates".into(),
                });
            } else if let Some(candidate) = candidate {
                let already_selected = selected
                    .iter()
                    .any(|item| item.object_id == candidate.object_id);
                let candidate_tokens = if already_selected {
                    0
                } else {
                    exact_token_count(candidate)?
                };
                if selected.len() + usize::from(!already_selected)
                    <= definition.budgets.max_context_items
                    && token_count
                        .checked_add(candidate_tokens)
                        .is_some_and(|count| count <= definition.budgets.max_context_tokens)
                {
                    token_count += candidate_tokens;
                    optional_selected.push(candidate.object_id.clone());
                    if !already_selected {
                        selected.push((*candidate).clone());
                    }
                } else {
                    omissions.push(ContextOmission {
                        role: role.clone(),
                        reason: "context_budget_exhausted".into(),
                    });
                }
            } else {
                omissions.push(ContextOmission {
                    role: role.clone(),
                    reason: if candidates.is_empty() {
                        "no_authorized_candidate".into()
                    } else {
                        "minimum_consistency_unavailable".into()
                    },
                });
            }
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
            manifest_id: stable_id(
                "ctx",
                &(
                    identity.tenant_id.as_str(),
                    atxn_id,
                    definition.task_type.as_str(),
                    definition.version,
                ),
            )?,
            tenant_id: identity.tenant_id.clone(),
            atxn_id: atxn_id.into(),
            task_definition: format!("{}:v{}", definition.task_type, definition.version),
            policy_epoch: identity.policy_epoch,
            required_role_coverage: required_coverage,
            optional_selected,
            omissions,
            conflicts: vec![],
            token_count,
            tokenizer: CONTEXT_TOKENIZER.into(),
            ranking_trace,
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
    request: &str,
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
    candidates.sort_by_key(|item| {
        (
            std::cmp::Reverse(item.authority.rank()),
            std::cmp::Reverse(item.consistency_class),
            std::cmp::Reverse(relevance_score(item, request)),
            std::cmp::Reverse(item.recorded_at),
            item.object_id.clone(),
        )
    });
    candidates
}

fn ensure_unambiguous(role: &str, candidates: &[&MemoryObject], request: &str) -> Result<()> {
    if is_ambiguous(candidates, request) {
        return Err(AmosError::Conflict(format!(
            "required context role {role} has ambiguous governing candidates"
        )));
    }
    Ok(())
}

fn is_ambiguous(candidates: &[&MemoryObject], request: &str) -> bool {
    let Some(first) = candidates.first() else {
        return false;
    };
    let top_rank = (
        first.authority.rank(),
        first.consistency_class,
        relevance_score(first, request),
    );
    candidates.iter().skip(1).any(|candidate| {
        (
            candidate.authority.rank(),
            candidate.consistency_class,
            relevance_score(candidate, request),
        ) == top_rank
            && candidate.content_hash != first.content_hash
    })
}

fn append_ranking_trace(
    trace: &mut Vec<ContextRankingEntry>,
    role: &str,
    candidates: &[&MemoryObject],
    request: &str,
    selected: Option<&MemoryObject>,
    minimum: ConsistencyClass,
) {
    for candidate in candidates {
        let is_selected = selected.is_some_and(|value| value.object_id == candidate.object_id);
        trace.push(ContextRankingEntry {
            role: role.into(),
            object_id: candidate.object_id.clone(),
            authority_rank: candidate.authority.rank(),
            consistency_class: candidate.consistency_class,
            relevance_score: relevance_score(candidate, request),
            selected: is_selected,
            reason: if candidate.consistency_class < minimum {
                format!("below_minimum_consistency_{minimum:?}")
            } else if is_selected {
                "selected".into()
            } else {
                "lower_rank_or_budget_omission".into()
            },
        });
    }
}

fn relevance_score(item: &MemoryObject, request: &str) -> u64 {
    let searchable =
        format!("{} {} {}", item.logical_key, item.summary, item.content).to_lowercase();
    request
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .map(str::to_lowercase)
        .filter(|term| searchable.contains(term))
        .count() as u64
}

fn exact_token_count(item: &MemoryObject) -> Result<usize> {
    let content = serde_json::to_string(&item.content)?;
    count_lexical_tokens(&item.summary)
        .checked_add(count_lexical_tokens(&content))
        .ok_or_else(|| AmosError::Validation("context token count overflow".into()))
}

fn count_lexical_tokens(value: &str) -> usize {
    let mut count = 0;
    let mut in_word = false;
    for character in value.chars() {
        if character.is_alphanumeric() || character == '_' {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else {
            in_word = false;
            if !character.is_whitespace() {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use serde_json::json;

    use super::*;
    use crate::{
        domain::{Authority, Budgets, RiskClass},
        memory::MemoryService,
        policy::PolicyEngine,
        store::Store,
    };

    fn identity() -> Identity {
        Identity {
            tenant_id: "tenant".into(),
            subject_id: "analyst".into(),
            roles: BTreeSet::from(["analyst".into()]),
            groups: BTreeSet::new(),
            permissions: BTreeSet::new(),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 1,
        }
    }

    fn definition() -> TaskDefinition {
        TaskDefinition {
            tenant_id: "tenant".into(),
            task_type: "test".into(),
            version: 1,
            status: "approved".into(),
            risk_class: RiskClass::Internal,
            required_roles: BTreeMap::from([("active_schema".into(), MemoryType::Schema)]),
            optional_roles: BTreeMap::from([("prior_incident".into(), MemoryType::PriorAnalysis)]),
            minimum_consistency: BTreeMap::from([("active_schema".into(), ConsistencyClass::C1)]),
            allowed_tools: BTreeSet::new(),
            claim_types: BTreeSet::new(),
            verifier_profile: "test.v1".into(),
            publication_policy: "local".into(),
            budgets: Budgets::default(),
            artifact_schema: "test.v1".into(),
            effective_start: Some(Utc::now() - Duration::hours(1)),
            effective_end: Some(Utc::now() + Duration::hours(1)),
        }
    }

    fn memory(
        logical_key: &str,
        memory_type: MemoryType,
        role: &str,
        summary: &str,
        consistency: ConsistencyClass,
    ) -> MemoryObject {
        let mut object = MemoryObject::new(
            "tenant",
            logical_key,
            memory_type,
            summary,
            json!({"role":role}),
            "source",
            logical_key,
            Authority::OwnerApproved,
        )
        .unwrap();
        object.consistency_class = consistency;
        object
    }

    #[test]
    fn compiler_enforces_consistency_exact_budget_and_ranking_trace() {
        let store = Store::in_memory().unwrap();
        let compiler = ContextCompiler::new(MemoryService::new(store.clone(), PolicyEngine));
        let low = memory(
            "schema:low",
            MemoryType::Schema,
            "active_schema",
            "schema",
            ConsistencyClass::C0,
        );
        store.write_memory(&low).unwrap();
        assert!(matches!(
            compiler.compile(
                &identity(),
                "atxn",
                "schema",
                &definition(),
                Utc::now() - Duration::minutes(1),
                Utc::now(),
            ),
            Err(AmosError::RequiredRoleMissing(_))
        ));

        let schema = memory(
            "schema:current",
            MemoryType::Schema,
            "active_schema",
            "schema",
            ConsistencyClass::C1,
        );
        let optional = memory(
            "incident:large",
            MemoryType::PriorAnalysis,
            "prior_incident",
            &"incident ".repeat(500),
            ConsistencyClass::C0,
        );
        store.write_memory(&schema).unwrap();
        store.write_memory(&optional).unwrap();
        let mut definition = definition();
        definition.budgets.max_context_tokens = exact_token_count(&schema).unwrap();
        let manifest = compiler
            .compile(
                &identity(),
                "atxn",
                "schema incident",
                &definition,
                Utc::now() - Duration::minutes(1),
                Utc::now(),
            )
            .unwrap();
        assert_eq!(manifest.tokenizer, CONTEXT_TOKENIZER);
        assert_eq!(manifest.token_count, exact_token_count(&schema).unwrap());
        assert!(manifest.optional_selected.is_empty());
        assert!(
            manifest
                .omissions
                .iter()
                .any(|omission| omission.reason == "context_budget_exhausted")
        );
        assert!(
            manifest
                .ranking_trace
                .iter()
                .any(|entry| entry.object_id == schema.object_id && entry.selected)
        );
        assert!(
            manifest
                .ranking_trace
                .iter()
                .any(|entry| entry.object_id == low.object_id
                    && entry.reason.starts_with("below_minimum_consistency"))
        );
    }

    #[test]
    fn compiler_rejects_ambiguous_required_governing_candidates() {
        let store = Store::in_memory().unwrap();
        for key in ["schema:a", "schema:b"] {
            let mut candidate = memory(
                key,
                MemoryType::Schema,
                "active_schema",
                "schema",
                ConsistencyClass::C1,
            );
            candidate.content = json!({"role":"active_schema","version":key});
            candidate.content_hash = crate::domain::content_hash(&candidate.content).unwrap();
            store.write_memory(&candidate).unwrap();
        }
        let compiler = ContextCompiler::new(MemoryService::new(store, PolicyEngine));
        assert!(matches!(
            compiler.compile(
                &identity(),
                "atxn",
                "schema",
                &definition(),
                Utc::now() - Duration::minutes(1),
                Utc::now(),
            ),
            Err(AmosError::Conflict(message)) if message.contains("ambiguous")
        ));
    }
}
