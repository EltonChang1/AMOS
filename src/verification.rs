use std::{collections::BTreeSet, ops::ControlFlow};

use chrono::Utc;
use sqlparser::{
    ast::{
        BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, Query,
        Select, SelectItem, SetExpr, Statement, Value, Visit, Visitor,
    },
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
                    && statements
                        .first()
                        .is_some_and(|statement| matches!(statement, Statement::Query(_)))
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
                    references
                        .identifiers
                        .contains(&old.to_lowercase())
                        .then(|| {
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
        let blocked: BTreeSet<_> = schemas
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
            .map(str::to_lowercase)
            .collect();
        let blocked_used = references
            .identifiers
            .iter()
            .find(|column| blocked.contains(*column));
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
        let where_predicates = parsed
            .as_ref()
            .ok()
            .map(|statements| collect_where_predicates(statements))
            .unwrap_or_default();
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
            .find(|filter| !where_satisfies_filter(&where_predicates, filter));
        record(
            &mut checks,
            "METRIC_FILTERS",
            missing_filter.map_or(Ok(()), |filter| {
                Err(format!("required metric filter missing: {filter}"))
            }),
            &mut errors,
        );
        let needs_denominator = matches!(
            step.expected_output_schema.as_str(),
            "rate_comparison.v1" | "concentration.v1" | "timeseries.v1"
        );
        if needs_denominator {
            record(
                &mut checks,
                "RESULT_DENOMINATOR",
                if references.has_count_star {
                    Ok(())
                } else {
                    Err("rate queries require COUNT(*) denominator".into())
                },
                &mut errors,
            );
        }
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

fn collect_where_predicates(statements: &[Statement]) -> BTreeSet<String> {
    let mut predicates = BTreeSet::new();
    for statement in statements {
        if let Statement::Query(query) = statement {
            collect_query_predicates(query, &mut predicates);
        }
    }
    predicates
}

fn collect_query_predicates(query: &Query, predicates: &mut BTreeSet<String>) {
    match query.body.as_ref() {
        SetExpr::Select(select) => {
            if let Some(selection) = &select.selection {
                flatten_predicate_keys(selection, predicates);
            }
        }
        SetExpr::Query(inner) => collect_query_predicates(inner, predicates),
        SetExpr::SetOperation { left, right, .. } => {
            if let SetExpr::Select(select) = left.as_ref()
                && let Some(selection) = &select.selection
            {
                flatten_predicate_keys(selection, predicates);
            }
            if let SetExpr::Select(select) = right.as_ref()
                && let Some(selection) = &select.selection
            {
                flatten_predicate_keys(selection, predicates);
            }
            if let SetExpr::Query(inner) = left.as_ref() {
                collect_query_predicates(inner, predicates);
            }
            if let SetExpr::Query(inner) = right.as_ref() {
                collect_query_predicates(inner, predicates);
            }
        }
        _ => {}
    }
}

fn flatten_predicate_keys(expression: &Expr, predicates: &mut BTreeSet<String>) {
    match expression {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            flatten_predicate_keys(left, predicates);
            flatten_predicate_keys(right, predicates);
        }
        Expr::Nested(inner) => flatten_predicate_keys(inner, predicates),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            if let Some(key) = equality_key(left, right) {
                predicates.insert(key);
            }
        }
        _ => {}
    }
}

fn equality_key(left: &Expr, right: &Expr) -> Option<String> {
    let left_id = identifier_name(left)?;
    let right_value = literal_key(right)?;
    Some(format!("{left_id}={right_value}"))
}

fn identifier_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Identifier(identifier) => Some(identifier.value.to_lowercase()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|part| part.value.to_lowercase()),
        _ => None,
    }
}

fn literal_key(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(text) | Value::DoubleQuotedString(text) => {
                Some(format!("'{text}'"))
            }
            Value::Number(number, _) => Some(number.clone()),
            Value::Boolean(flag) => Some(if *flag { "true" } else { "false" }.into()),
            _ => None,
        },
        Expr::UnaryOp {
            op: sqlparser::ast::UnaryOperator::Minus,
            expr,
        } => Some(format!("-{}", literal_key(expr)?)),
        Expr::UnaryOp {
            op: sqlparser::ast::UnaryOperator::Plus,
            expr,
        } => literal_key(expr),
        _ => None,
    }
}

fn where_satisfies_filter(predicates: &BTreeSet<String>, filter: &str) -> bool {
    let Ok(expressions) =
        Parser::parse_sql(&GenericDialect {}, &format!("SELECT 1 WHERE {filter}"))
    else {
        return false;
    };
    let required = collect_where_predicates(&expressions);
    !required.is_empty() && required.iter().all(|key| predicates.contains(key))
}

#[derive(Default)]
struct SchemaReferences {
    relations: BTreeSet<String>,
    identifiers: BTreeSet<String>,
    aliases: BTreeSet<String>,
    has_count_star: bool,
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
            Expr::Function(function) => {
                let name = function.name.to_string().to_lowercase();
                if name == "count" {
                    match &function.args {
                        FunctionArguments::List(list)
                            if list.args.iter().any(|arg| {
                                matches!(
                                    arg,
                                    FunctionArg::Unnamed(FunctionArgExpr::Wildcard)
                                        | FunctionArg::Unnamed(FunctionArgExpr::QualifiedWildcard(
                                            _
                                        ))
                                )
                            }) =>
                        {
                            self.has_count_star = true;
                        }
                        FunctionArguments::None => {
                            // COUNT without args is not a valid denominator form.
                        }
                        _ => {}
                    }
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
    fn metric_filters_require_where_predicates_not_string_literals() {
        let predicates = collect_where_predicates(
            &Parser::parse_sql(
                &GenericDialect {},
                "SELECT 'environment = ''production''' AS note FROM payment_events WHERE is_test_account = 0",
            )
            .unwrap(),
        );
        assert!(where_satisfies_filter(&predicates, "is_test_account = 0"));
        assert!(!where_satisfies_filter(
            &predicates,
            "environment = 'production'"
        ));
    }

    #[test]
    fn metric_filters_accept_equivalent_where_equality() {
        let predicates = collect_where_predicates(
            &Parser::parse_sql(
                &GenericDialect {},
                "SELECT 1 FROM payment_events WHERE environment = 'production' AND is_test_account = 0",
            )
            .unwrap(),
        );
        assert!(where_satisfies_filter(
            &predicates,
            "environment = 'production'"
        ));
        assert!(where_satisfies_filter(&predicates, "is_test_account = 0"));
    }
}
