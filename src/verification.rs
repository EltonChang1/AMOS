use std::{collections::BTreeSet, ops::ControlFlow};

use chrono::Utc;
use sqlparser::{
    ast::{Expr, ObjectName, Select, SelectItem, Visit, Visitor},
    dialect::GenericDialect,
    parser::Parser,
};

use crate::{
    domain::{
        Claim, ContextManifest, DependencyEdge, Identity, Outcome, PlanStep, TaskDefinition,
        VerificationCheck, VerificationRecord, content_hash, new_id,
    },
    policy::PolicyEngine,
};

#[derive(Debug, Clone, Default)]
pub struct Verifier {
    policy: PolicyEngine,
}

impl Verifier {
    pub fn verify_step(
        &self,
        identity: &Identity,
        definition: &TaskDefinition,
        manifest: &ContextManifest,
        step: &PlanStep,
    ) -> VerificationRecord {
        let mut checks = vec![];
        let mut warnings = vec![];
        let mut errors = vec![];
        let relations = step
            .parameters
            .get("relations")
            .and_then(|v| v.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        record(
            &mut checks,
            "TOOL_POLICY",
            self.policy
                .authorize_tool(identity, definition, &step.tool, &relations)
                .map_err(|e| e.to_string()),
            &mut errors,
        );
        let sql = step
            .parameters
            .get("sql")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let parsed = Parser::parse_sql(&GenericDialect {}, sql);
        let read_only = parsed
            .as_ref()
            .map(|statements| {
                statements.len() == 1
                    && statements.first().is_some_and(|statement| {
                        matches!(statement, sqlparser::ast::Statement::Query(_))
                    })
            })
            .unwrap_or(false);
        record(
            &mut checks,
            "SQL_READ_ONLY",
            if read_only {
                Ok(())
            } else {
                Err("only one read-only SELECT is allowed".into())
            },
            &mut errors,
        );
        if let Err(ref error) = parsed {
            errors.push(format!("SQL parse failed: {error}"));
        }

        let normalized = normalize(sql);
        let schemas: Vec<_> = manifest
            .selected_objects
            .iter()
            .filter(|object| matches!(object.memory_type, crate::domain::MemoryType::Schema))
            .collect();
        let mut references = SchemaReferences::default();
        if let Ok(statements) = &parsed {
            let _ = statements.visit(&mut references);
        }
        let allowed_tables: BTreeSet<_> = schemas
            .iter()
            .filter_map(|schema| schema.content.get("table").and_then(|value| value.as_str()))
            .map(str::to_lowercase)
            .collect();
        let allowed_columns: BTreeSet<_> = schemas
            .iter()
            .flat_map(|schema| {
                schema
                    .content
                    .get("columns")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str())
            })
            .map(str::to_lowercase)
            .collect();
        let permitted_repair = schemas.iter().find_map(|schema| {
            schema
                .content
                .get("renamed_columns")?
                .as_object()?
                .iter()
                .find_map(|(old, new)| {
                    normalized.contains(&normalize(old)).then(|| {
                        format!(
                            "COLUMN_SUPERSEDED:{old}:{}",
                            new.as_str().unwrap_or_default()
                        )
                    })
                })
        });
        let unknown_table = references
            .relations
            .iter()
            .find(|table| !allowed_tables.contains(*table));
        record(
            &mut checks,
            "SCHEMA_TABLES",
            unknown_table.map_or(Ok(()), |table| Err(format!("unknown table: {table}"))),
            &mut errors,
        );
        let unknown_column =
            references
                .identifiers
                .difference(&references.aliases)
                .find(|column| {
                    !allowed_columns.contains(*column)
                        && !permitted_repair.as_deref().is_some_and(|repair| {
                            repair.starts_with(&format!("COLUMN_SUPERSEDED:{column}:"))
                        })
                });
        record(
            &mut checks,
            "SCHEMA_COLUMNS",
            unknown_column.map_or(Ok(()), |column| {
                Err(format!("unknown or superseded column: {column}"))
            }),
            &mut errors,
        );
        let blocked: Vec<_> = schemas
            .iter()
            .flat_map(|schema| {
                schema
                    .content
                    .get("blocked_columns")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
            })
            .collect();
        let blocked_used = blocked
            .iter()
            .find(|column| normalized.contains(&normalize(column)));
        record(
            &mut checks,
            "SCHEMA_BLOCKED_COLUMNS",
            blocked_used.map_or(Ok(()), |column| {
                Err(format!("blocked column referenced: {column}"))
            }),
            &mut errors,
        );
        let metrics: Vec<_> = manifest
            .selected_objects
            .iter()
            .filter(|object| {
                matches!(
                    object.memory_type,
                    crate::domain::MemoryType::SemanticDefinition
                )
            })
            .collect();
        let missing_filter = metrics
            .iter()
            .flat_map(|metric| {
                metric
                    .content
                    .get("required_filters")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
            })
            .find(|filter| !normalized.contains(&normalize(filter)));
        record(
            &mut checks,
            "METRIC_FILTERS",
            missing_filter.map_or(Ok(()), |filter| {
                Err(format!("required metric filter missing: {filter}"))
            }),
            &mut errors,
        );
        for state in manifest.selected_objects.iter().filter(|object| {
            matches!(
                object.memory_type,
                crate::domain::MemoryType::DataState | crate::domain::MemoryType::StreamState
            )
        }) {
            if let Some(warning) = state
                .content
                .get("freshness_warning")
                .and_then(|v| v.as_str())
            {
                warnings.push(warning.into());
            }
        }
        if permitted_repair.is_some() {
            checks.push(VerificationCheck {
                rule_id: "COLUMN_SUPERSEDED".into(),
                outcome: Outcome::Repair,
                message: permitted_repair.clone(),
            });
        }
        let outcome = if !errors.is_empty() {
            Outcome::Reject
        } else if permitted_repair.is_some() {
            Outcome::Repair
        } else if !warnings.is_empty() {
            Outcome::Warning
        } else {
            Outcome::Pass
        };
        VerificationRecord {
            verification_id: new_id("ver"),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: manifest.atxn_id.clone(),
            execution_id: None,
            verifier_profile: definition.verifier_profile.clone(),
            profile_version: 1,
            outcome,
            checks,
            warnings,
            errors,
            permitted_repair,
            input_hash: content_hash(step),
            created_at: Utc::now(),
        }
    }

    pub fn repair_step(&self, step: &PlanStep, repair: &str) -> Option<PlanStep> {
        let mut parts = repair.splitn(3, ':');
        if parts.next()? != "COLUMN_SUPERSEDED" {
            return None;
        }
        let old = parts.next()?;
        let new = parts.next()?;
        let mut repaired = step.clone();
        let sql = repaired.parameters.get("sql")?.as_str()?.replace(old, new);
        repaired.parameters["sql"] = serde_json::Value::String(sql);
        Some(repaired)
    }

    pub fn verify_claims(
        &self,
        tenant: &str,
        atxn_id: &str,
        profile: &str,
        claims: &[Claim],
        edges: &[DependencyEdge],
    ) -> VerificationRecord {
        let mut checks = vec![];
        let mut errors = vec![];
        let mut warnings = vec![];
        for claim in claims {
            let relations: BTreeSet<_> = edges
                .iter()
                .filter(|edge| edge.from.id == claim.claim_id)
                .map(|edge| edge.relation.as_str())
                .collect();
            let numeric = matches!(
                claim.claim_type.as_str(),
                "metric_value" | "metric_comparison" | "concentration"
            );
            if numeric {
                let required = [
                    "computed_by",
                    "governed_by_metric",
                    "governed_by_schema",
                    "scoped_to_data_state",
                ];
                for relation in required {
                    if !relations.contains(relation) {
                        errors.push(format!("claim {} missing {relation}", claim.claim_id));
                    }
                }
            }
            if claim.claim_type == "operational_recommendation" || claim.claim_type == "causal" {
                warnings.push(format!("claim {} requires human review", claim.claim_id));
            }
        }
        checks.push(VerificationCheck {
            rule_id: "CLAIM_SUPPORT".into(),
            outcome: if errors.is_empty() {
                Outcome::Pass
            } else {
                Outcome::Reject
            },
            message: None,
        });
        checks.push(VerificationCheck {
            rule_id: "REVIEW_BOUNDARY".into(),
            outcome: if warnings.is_empty() {
                Outcome::Pass
            } else {
                Outcome::NeedsReview
            },
            message: None,
        });
        let outcome = if !errors.is_empty() {
            Outcome::Reject
        } else if !warnings.is_empty() {
            Outcome::NeedsReview
        } else {
            Outcome::Pass
        };
        VerificationRecord {
            verification_id: new_id("ver"),
            tenant_id: tenant.into(),
            atxn_id: atxn_id.into(),
            execution_id: None,
            verifier_profile: profile.into(),
            profile_version: 1,
            outcome,
            checks,
            warnings,
            errors,
            permitted_repair: None,
            input_hash: content_hash(&claims),
            created_at: Utc::now(),
        }
    }
}

fn record(
    checks: &mut Vec<VerificationCheck>,
    rule: &str,
    result: std::result::Result<(), String>,
    errors: &mut Vec<String>,
) {
    match result {
        Ok(()) => checks.push(VerificationCheck {
            rule_id: rule.into(),
            outcome: Outcome::Pass,
            message: None,
        }),
        Err(error) => {
            errors.push(error.clone());
            checks.push(VerificationCheck {
                rule_id: rule.into(),
                outcome: Outcome::Reject,
                message: Some(error),
            })
        }
    }
}
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Default)]
struct SchemaReferences {
    relations: BTreeSet<String>,
    identifiers: BTreeSet<String>,
    aliases: BTreeSet<String>,
}

impl Visitor for SchemaReferences {
    type Break = ();

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        self.relations.insert(relation.to_string().to_lowercase());
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        for item in &select.projection {
            if let SelectItem::ExprWithAlias { alias, .. } = item {
                self.aliases.insert(alias.value.to_lowercase());
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::Identifier(identifier) => {
                self.identifiers.insert(identifier.value.to_lowercase());
            }
            Expr::CompoundIdentifier(identifiers) => {
                if let Some(identifier) = identifiers.last() {
                    self.identifiers.insert(identifier.value.to_lowercase());
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalize_ignores_sql_spacing() {
        assert_eq!(normalize("is_test = FALSE"), normalize("is_test=false"));
    }
}
