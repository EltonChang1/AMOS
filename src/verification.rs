use std::{collections::BTreeSet, ops::ControlFlow};

use chrono::Utc;
use sqlparser::{
    ast::{BinaryOperator, Expr, ObjectName, Query, Select, SelectItem, SetExpr, Visit, Visitor},
    dialect::GenericDialect,
    parser::Parser,
};

use crate::{
    Result,
    domain::{
        Artifact, Claim, ContextManifest, DependencyEdge, ExecutionRecord, Identity, Outcome,
        PlanStep, TaskDefinition, VerificationCheck, VerificationRecord, content_hash, stable_id,
    },
    policy::PolicyEngine,
    workers::ChartWorker,
};

#[derive(Debug, Clone, Default)]
pub struct Verifier {
    policy: PolicyEngine,
}

pub struct ClaimVerificationRequest<'a> {
    pub tenant: &'a str,
    pub atxn_id: &'a str,
    pub profile: &'a str,
    pub artifact: &'a Artifact,
    pub manifest: &'a ContextManifest,
    pub claims: &'a [Claim],
    pub edges: &'a [DependencyEdge],
    pub executions: &'a [ExecutionRecord],
    pub verifications: &'a [VerificationRecord],
}

impl Verifier {
    pub fn verify_step(
        &self,
        identity: &Identity,
        definition: &TaskDefinition,
        manifest: &ContextManifest,
        step: &PlanStep,
    ) -> Result<VerificationRecord> {
        let mut checks = vec![];
        let mut warnings = vec![];
        let mut errors = vec![];
        let declared_relations = step
            .parameters
            .get("relations")
            .and_then(|value| value.as_array());
        let relations_are_valid = declared_relations.is_some_and(|values| {
            !values.is_empty()
                && values
                    .iter()
                    .all(|value| value.as_str().is_some_and(|text| !text.trim().is_empty()))
        });
        let relations = declared_relations
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        record(
            &mut checks,
            "STEP_RELATIONS",
            relations_are_valid
                .then_some(())
                .ok_or_else(|| "step must declare a non-empty string relation list".into()),
            &mut errors,
        );
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
        if let Ok(statements) = &parsed
            && statements.visit(&mut references).is_break()
        {
            errors.push("SQL schema reference traversal stopped unexpectedly".into());
        }
        let allowed_functions =
            BTreeSet::from(["count".to_string(), "substr".to_string(), "sum".to_string()]);
        let unsupported_functions = references
            .functions
            .difference(&allowed_functions)
            .cloned()
            .collect::<Vec<_>>();
        record(
            &mut checks,
            "SQL_SUPPORTED_SUBSET",
            if sql.len() > 32 * 1024
                || references.has_join
                || references.has_cte
                || references.has_set_operation
                || references.has_subquery
                || !unsupported_functions.is_empty()
            {
                Err(format!(
                    "query uses unsupported SQL structure or functions: {}",
                    unsupported_functions.join(",")
                ))
            } else {
                Ok(())
            },
            &mut errors,
        );
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
                    new.as_str()
                        .filter(|replacement| !replacement.trim().is_empty())
                        .filter(|_| normalized.contains(&normalize(old)))
                        .map(|replacement| format!("COLUMN_SUPERSEDED:{old}:{replacement}"))
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
        let payment_events_used = references.relations.contains("payment_events");
        record(
            &mut checks,
            "SQL_TIME_BOUNDS",
            if !payment_events_used
                || (references.has_time_lower_bound && references.has_time_upper_bound)
            {
                Ok(())
            } else {
                Err(
                    "payment_events queries require parsed lower and upper event_time bounds"
                        .into(),
                )
            },
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
        let input_hash = content_hash(step)?;
        Ok(VerificationRecord {
            verification_id: stable_id(
                "ver",
                &serde_json::json!({
                    "tenant_id": identity.tenant_id,
                    "atxn_id": manifest.atxn_id,
                    "profile": definition.verifier_profile,
                    "input_hash": input_hash,
                }),
            )?,
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
            input_hash,
            created_at: Utc::now(),
        })
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
        request: &ClaimVerificationRequest<'_>,
    ) -> Result<VerificationRecord> {
        let tenant = request.tenant;
        let atxn_id = request.atxn_id;
        let profile = request.profile;
        let artifact = request.artifact;
        let manifest = request.manifest;
        let claims = request.claims;
        let edges = request.edges;
        let executions = request.executions;
        let verifications = request.verifications;
        let mut checks = vec![];
        let mut errors = vec![];
        let mut warnings = vec![];
        let execution_ids = executions
            .iter()
            .map(|execution| (execution.execution_id.as_str(), execution))
            .collect::<std::collections::BTreeMap<_, _>>();
        let verification_ids = verifications
            .iter()
            .map(|verification| verification.verification_id.as_str())
            .collect::<BTreeSet<_>>();
        let memory_ids = manifest
            .selected_objects
            .iter()
            .map(|memory| memory.object_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut reference_errors = vec![];
        let mut support_errors = vec![];
        let mut numeric_errors = vec![];
        for claim in claims {
            if claim.tenant_id != tenant || claim.artifact_id != artifact.artifact_id {
                reference_errors.push(format!(
                    "claim {} crosses tenant or artifact scope",
                    claim.claim_id
                ));
            }
            let claim_edges = edges
                .iter()
                .filter(|edge| edge.from.id == claim.claim_id)
                .collect::<Vec<_>>();
            let relations: BTreeSet<_> = claim_edges
                .iter()
                .map(|edge| edge.relation.as_str())
                .collect();
            for edge in &claim_edges {
                if edge.tenant_id != tenant
                    || edge.from.endpoint_type != "claim"
                    || (edge.to.endpoint_type == "execution"
                        && !execution_ids.contains_key(edge.to.id.as_str()))
                    || (edge.to.endpoint_type == "memory"
                        && !memory_ids.contains(edge.to.id.as_str()))
                {
                    reference_errors.push(format!(
                        "claim {} has an unresolved or cross-scope dependency {}",
                        claim.claim_id, edge.edge_id
                    ));
                }
            }
            for execution_id in &claim.support_execution_ids {
                let Some(execution) = execution_ids.get(execution_id.as_str()) else {
                    reference_errors.push(format!(
                        "claim {} references missing execution {execution_id}",
                        claim.claim_id
                    ));
                    continue;
                };
                if execution.tenant_id != tenant
                    || execution.atxn_id != atxn_id
                    || !claim_edges.iter().any(|edge| {
                        edge.relation == "computed_by"
                            && edge.to.endpoint_type == "execution"
                            && edge.to.id == *execution_id
                    })
                {
                    reference_errors.push(format!(
                        "claim {} execution {execution_id} is not bound by a computed_by edge",
                        claim.claim_id
                    ));
                }
            }
            for verification_id in &claim.verification_ids {
                if !verification_ids.contains(verification_id.as_str()) {
                    reference_errors.push(format!(
                        "claim {} references missing verification {verification_id}",
                        claim.claim_id
                    ));
                }
            }
            let numeric = matches!(
                claim.claim_type.as_str(),
                "metric_value" | "metric_comparison" | "concentration"
            );
            if numeric {
                if claim.support_execution_ids.is_empty() || claim.verification_ids.is_empty() {
                    reference_errors.push(format!(
                        "numeric claim {} requires execution and verification references",
                        claim.claim_id
                    ));
                }
                let required = [
                    "computed_by",
                    "governed_by_metric",
                    "governed_by_schema",
                    "scoped_to_data_state",
                ];
                for relation in required {
                    if !relations.contains(relation) {
                        support_errors.push(format!("claim {} missing {relation}", claim.claim_id));
                    }
                }
                if let Err(error) = verify_numeric_claim(claim, &execution_ids) {
                    numeric_errors.push(error);
                }
            }
            if claim.claim_type == "operational_recommendation" || claim.claim_type == "causal" {
                warnings.push(format!("claim {} requires human review", claim.claim_id));
            }
        }
        let chart_result = verify_chart_binding(artifact, executions);
        record(
            &mut checks,
            "CLAIM_REFERENCES",
            join_errors(&reference_errors),
            &mut errors,
        );
        record(
            &mut checks,
            "CLAIM_SUPPORT",
            join_errors(&support_errors),
            &mut errors,
        );
        record(
            &mut checks,
            "NUMERIC_RECOMPUTATION",
            join_errors(&numeric_errors),
            &mut errors,
        );
        record(&mut checks, "CHART_DATA_BINDING", chart_result, &mut errors);
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
        let input_hash = content_hash(&serde_json::json!({
            "artifact": artifact,
            "manifest_id": manifest.manifest_id,
            "claims": claims,
            "edges": edges,
            "execution_hashes": executions.iter().map(|execution| {
                (&execution.execution_id, &execution.output_hash)
            }).collect::<Vec<_>>(),
            "verification_ids": verifications.iter().map(|verification| {
                &verification.verification_id
            }).collect::<Vec<_>>(),
        }))?;
        Ok(VerificationRecord {
            verification_id: stable_id(
                "ver",
                &serde_json::json!({
                    "tenant_id": tenant,
                    "atxn_id": atxn_id,
                    "profile": profile,
                    "input_hash": input_hash,
                }),
            )?,
            tenant_id: tenant.into(),
            atxn_id: atxn_id.into(),
            execution_id: None,
            verifier_profile: profile.into(),
            profile_version: 2,
            outcome,
            checks,
            warnings,
            errors,
            permitted_repair: None,
            input_hash,
            created_at: Utc::now(),
        })
    }
}

fn join_errors(errors: &[String]) -> std::result::Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn verify_numeric_claim(
    claim: &Claim,
    executions: &std::collections::BTreeMap<&str, &ExecutionRecord>,
) -> std::result::Result<(), String> {
    let execution = claim
        .support_execution_ids
        .first()
        .and_then(|execution_id| executions.get(execution_id.as_str()))
        .ok_or_else(|| {
            format!(
                "claim {} has no durable supporting execution",
                claim.claim_id
            )
        })?;
    match claim.claim_type.as_str() {
        "metric_comparison" => {
            let rows = execution
                .output
                .as_array()
                .ok_or_else(|| format!("claim {} supporting output is not rows", claim.claim_id))?;
            let current = rows
                .iter()
                .find(|row| {
                    row.get("period").and_then(serde_json::Value::as_str) == Some("current")
                })
                .ok_or_else(|| format!("claim {} has no current row", claim.claim_id))?;
            let baseline = rows
                .iter()
                .find(|row| {
                    row.get("period").and_then(serde_json::Value::as_str) == Some("baseline")
                })
                .ok_or_else(|| format!("claim {} has no baseline row", claim.claim_id))?;
            let current_rate = recompute_rate(current, &claim.claim_id)?;
            let baseline_rate = recompute_rate(baseline, &claim.claim_id)?;
            let claimed_current = finite_number(&claim.payload, "current_value", &claim.claim_id)?;
            let claimed_baseline =
                finite_number(&claim.payload, "baseline_value", &claim.claim_id)?;
            if (current_rate - claimed_current).abs() > 1e-12
                || (baseline_rate - claimed_baseline).abs() > 1e-12
            {
                return Err(format!(
                    "claim {} numeric values do not recompute from execution {}",
                    claim.claim_id, execution.execution_id
                ));
            }
            Ok(())
        }
        "concentration" => {
            let first = execution
                .output
                .as_array()
                .and_then(|rows| rows.first())
                .ok_or_else(|| format!("claim {} concentration output is empty", claim.claim_id))?;
            let rate = recompute_rate(first, &claim.claim_id)?;
            let reported_rate = finite_number(first, "failure_rate", &claim.claim_id)?;
            if &claim.payload != first || (rate - reported_rate).abs() > 1e-12 {
                return Err(format!(
                    "claim {} concentration payload does not match the top execution row",
                    claim.claim_id
                ));
            }
            Ok(())
        }
        "metric_value" => {
            let value = finite_number(&claim.payload, "value", &claim.claim_id)?;
            if value.is_finite() {
                Ok(())
            } else {
                Err(format!(
                    "claim {} metric value is not finite",
                    claim.claim_id
                ))
            }
        }
        _ => Ok(()),
    }
}

fn recompute_rate(row: &serde_json::Value, claim_id: &str) -> std::result::Result<f64, String> {
    let failures = row
        .get("failures")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("claim {claim_id} has no integer failures"))?;
    let attempts = row
        .get("attempts")
        .and_then(serde_json::Value::as_u64)
        .filter(|attempts| *attempts > 0)
        .ok_or_else(|| format!("claim {claim_id} has no positive integer attempts"))?;
    if failures > attempts {
        return Err(format!("claim {claim_id} failures exceed attempts"));
    }
    Ok(failures as f64 / attempts as f64)
}

fn finite_number(
    value: &serde_json::Value,
    field: &str,
    claim_id: &str,
) -> std::result::Result<f64, String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("claim {claim_id} field {field} is not a finite number"))
}

fn verify_chart_binding(
    artifact: &Artifact,
    executions: &[ExecutionRecord],
) -> std::result::Result<(), String> {
    let timeseries = executions
        .iter()
        .find(|execution| execution.step_id == "timeseries")
        .ok_or_else(|| "timeseries execution is missing".to_string())?;
    let rows = timeseries
        .output
        .as_array()
        .ok_or_else(|| "timeseries execution output is not rows".to_string())?;
    let points = rows
        .iter()
        .map(|row| {
            let hour = row
                .get("hour")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "timeseries row has no hour".to_string())?;
            let value = row
                .get("failure_rate")
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
                .ok_or_else(|| "timeseries row has no finite failure rate".to_string())?;
            Ok((hour.to_string(), value))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    let (svg, hash) = ChartWorker
        .timeseries_svg(&points)
        .map_err(|error| error.to_string())?;
    if !artifact.content.contains(&hash) || !artifact.content.contains(&svg) {
        return Err("artifact chart is not bound to the timeseries execution data".into());
    }
    Ok(())
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
    functions: BTreeSet<String>,
    has_join: bool,
    has_cte: bool,
    has_set_operation: bool,
    has_subquery: bool,
    query_count: usize,
    has_time_lower_bound: bool,
    has_time_upper_bound: bool,
}

impl Visitor for SchemaReferences {
    type Break = ();

    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        self.relations.insert(relation.to_string().to_lowercase());
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        self.has_join |= select.from.iter().any(|table| !table.joins.is_empty());
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
            Expr::Function(function) => {
                self.functions
                    .insert(function.name.to_string().to_lowercase());
            }
            Expr::Subquery(_) => {
                self.has_subquery = true;
            }
            Expr::BinaryOp { left, op, right } => {
                let left_is_time = expression_identifier(left).as_deref() == Some("event_time");
                let right_is_time = expression_identifier(right).as_deref() == Some("event_time");
                if left_is_time {
                    self.has_time_lower_bound |=
                        matches!(op, BinaryOperator::Gt | BinaryOperator::GtEq);
                    self.has_time_upper_bound |=
                        matches!(op, BinaryOperator::Lt | BinaryOperator::LtEq);
                } else if right_is_time {
                    self.has_time_lower_bound |=
                        matches!(op, BinaryOperator::Lt | BinaryOperator::LtEq);
                    self.has_time_upper_bound |=
                        matches!(op, BinaryOperator::Gt | BinaryOperator::GtEq);
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        self.query_count += 1;
        if self.query_count > 1 {
            self.has_subquery = true;
        }
        self.has_cte |= query.with.is_some();
        self.has_set_operation |= matches!(query.body.as_ref(), SetExpr::SetOperation { .. });
        ControlFlow::Continue(())
    }
}

fn expression_identifier(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Identifier(identifier) => Some(identifier.value.to_lowercase()),
        Expr::CompoundIdentifier(identifiers) => identifiers
            .last()
            .map(|identifier| identifier.value.to_lowercase()),
        _ => None,
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
