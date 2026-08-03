use std::sync::Arc;

use amos::{
    AmosError, Result,
    model::{
        ModelDescriptor, ModelProvider, ModelPurpose, ModelRequest, ModelResponse,
        NARRATIVE_SCHEMA_VERSION, PLAN_SCHEMA_VERSION,
    },
    privacy::ModelRouteClass,
};
use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Debug, Default)]
pub struct TestModelProvider;

#[async_trait]
impl ModelProvider for TestModelProvider {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor {
            provider: "stub".into(),
            model: "test-gemma".into(),
            route_class: ModelRouteClass::Local,
        }
    }

    async fn generate_structured(&self, request: ModelRequest) -> Result<ModelResponse> {
        let output = match request.purpose {
            ModelPurpose::Plan => plan_from_contract(&request.payload)?,
            ModelPurpose::Narrative => narrative_from_context(&request.payload)?,
        };
        Ok(ModelResponse {
            output_text: serde_json::to_string(&output)
                .map_err(|_| AmosError::Serialization("test model response".into()))?,
            input_tokens: 100,
            output_tokens: 50,
            provider_invocation_id: Some(format!("stub-{}", request.invocation_id)),
        })
    }
}

/// Build a valid plan from the pack-supplied SQL contract so any installed pack
/// can execute without a pack-specific mock branch.
fn plan_from_contract(payload: &Value) -> Result<Value> {
    let contract = &payload["sql_contract"];
    let relation = contract["relation"]
        .as_str()
        .ok_or_else(|| AmosError::Validation("test plan missing relation".into()))?;
    let time_field = contract["time_field"]
        .as_str()
        .ok_or_else(|| AmosError::Validation("test plan missing time_field".into()))?;
    let filters = contract["required_metric_filters"]
        .as_array()
        .ok_or_else(|| AmosError::Validation("test plan missing filters".into()))?
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" AND ");
    let rate_schema = payload["result_schemas"]["rate_comparison"]
        .as_array()
        .ok_or_else(|| AmosError::Validation("test plan missing rate schema".into()))?;
    let conc_schema = payload["result_schemas"]["concentration"]
        .as_array()
        .ok_or_else(|| AmosError::Validation("test plan missing concentration schema".into()))?;
    let ts_schema = payload["result_schemas"]["timeseries"]
        .as_array()
        .ok_or_else(|| AmosError::Validation("test plan missing timeseries schema".into()))?;

    let period = rate_schema[0].as_str().unwrap_or("period");
    let numerator = rate_schema[1].as_str().unwrap_or("churned_accounts");
    let denominator = rate_schema[2].as_str().unwrap_or("eligible_accounts");
    let rate = rate_schema[3].as_str().unwrap_or("churn_rate");
    let ts_label = ts_schema[0].as_str().unwrap_or("day");
    let ts_value = ts_schema[1].as_str().unwrap_or("churn_rate");

    let current_start = payload["time_window"]["current_start"]
        .as_str()
        .unwrap_or("2026-07-20T00:00:00Z");
    let current_bound = if time_field.ends_with("_date") {
        current_start.split('T').next().unwrap_or(current_start)
    } else {
        current_start
    };

    let all_bounds = &contract["required_time_bounds"]["rate_comparison"];
    let current_bounds = &contract["required_time_bounds"]["concentration"];
    let all_where = format!(
        "{} AND {} AND {}",
        all_bounds["lower"].as_str().unwrap_or_default(),
        all_bounds["upper"].as_str().unwrap_or_default(),
        filters
    );
    let current_where = format!(
        "{} AND {} AND {}",
        current_bounds["lower"].as_str().unwrap_or_default(),
        current_bounds["upper"].as_str().unwrap_or_default(),
        filters
    );

    // Payment-failure pack numerators count payment_failure churn events;
    // other packs count all churned flags.
    let task_type = payload["task"]["task_type"].as_str().unwrap_or_default();
    let numerator_expr = if task_type.contains("payment_failure") {
        "SUM(CASE WHEN churn_reason = 'payment_failure' THEN churned ELSE 0 END)"
    } else {
        "SUM(churned)"
    };

    let dim_cols: Vec<&str> = conc_schema
        .iter()
        .filter_map(Value::as_str)
        .take_while(|col| {
            !matches!(
                *col,
                "churned_accounts"
                    | "eligible_accounts"
                    | "churn_rate"
                    | "failures"
                    | "attempts"
                    | "failure_rate"
            )
        })
        .collect();
    let dim_select = dim_cols.join(", ");
    let dim_group = dim_cols.join(",");

    Ok(json!({
        "schema_version": PLAN_SCHEMA_VERSION,
        "summary": format!("Compare {task_type} rate, concentration, and trend."),
        "steps": [
            {
                "analysis_kind": "rate_comparison",
                "purpose": "Compare current and baseline rates",
                "sql": format!(
                    "SELECT CASE WHEN {time_field} >= '{current_bound}' THEN 'current' ELSE 'baseline' END AS {period}, \
                     {numerator_expr} AS {numerator}, COUNT(DISTINCT account_id) AS {denominator}, \
                     CAST({numerator_expr} AS REAL)/COUNT(DISTINCT account_id) AS {rate} \
                     FROM {relation} WHERE {all_where} GROUP BY {period} ORDER BY {period}"
                ),
                "relations": [relation],
                "expected_columns": rate_schema
            },
            {
                "analysis_kind": "concentration",
                "purpose": "Find the largest concentration",
                "sql": format!(
                    "SELECT {dim_select}, {numerator_expr} AS {numerator}, COUNT(DISTINCT account_id) AS {denominator}, \
                     CAST({numerator_expr} AS REAL)/COUNT(DISTINCT account_id) AS {rate} \
                     FROM {relation} WHERE {current_where} GROUP BY {dim_group} \
                     ORDER BY {numerator} DESC LIMIT 10"
                ),
                "relations": [relation],
                "expected_columns": conc_schema
            },
            {
                "analysis_kind": "timeseries",
                "purpose": "Show the daily trend",
                "sql": format!(
                    "SELECT {time_field} AS {ts_label}, \
                     CAST({numerator_expr} AS REAL)/COUNT(DISTINCT account_id) AS {ts_value} \
                     FROM {relation} WHERE {all_where} GROUP BY {time_field} ORDER BY {time_field}"
                ),
                "relations": [relation],
                "expected_columns": ts_schema
            }
        ]
    }))
}

fn narrative_from_context(payload: &Value) -> Result<Value> {
    let memory_id = |logical_key: &str| {
        payload["permitted_context"]
            .as_array()
            .and_then(|objects| {
                objects
                    .iter()
                    .find(|object| object["logical_key"].as_str() == Some(logical_key))
            })
            .and_then(|object| object["object_id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                AmosError::Validation(format!("test narrative context lacks {logical_key}"))
            })
    };
    let support_doc = memory_id("document:pricing_email_launch").or_else(|_| {
        payload["permitted_context"]
            .as_array()
            .and_then(|objects| {
                objects.iter().find(|object| {
                    object["logical_key"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("document:"))
                })
            })
            .and_then(|object| object["object_id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                AmosError::Validation("test narrative context lacks any document".into())
            })
    })?;
    let snapshot = memory_id("snapshot:subscription_events:2026-07-27").or_else(|_| {
        payload["permitted_context"]
            .as_array()
            .and_then(|objects| {
                objects.iter().find(|object| {
                    object["logical_key"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("snapshot:"))
                })
            })
            .and_then(|object| object["object_id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                AmosError::Validation("test narrative context lacks any snapshot".into())
            })
    })?;
    let review_policy = memory_id("policy:review:subscriptions").or_else(|_| {
        payload["permitted_context"]
            .as_array()
            .and_then(|objects| {
                objects.iter().find(|object| {
                    object["logical_key"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("policy:review:"))
                })
            })
            .and_then(|object| object["object_id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                AmosError::Validation("test narrative context lacks review policy".into())
            })
    })?;
    Ok(json!({
        "schema_version": NARRATIVE_SCHEMA_VERSION,
        "title": "Governed metric review",
        "executive_summary": "The verified movement merits investigation. The largest verified concentration is {{fact:concentration.top}} Temporal association is not proven causal.",
        "finding_order": [
            "metric.rate_change",
            "concentration.top",
            "trend.daily"
        ],
        "sections": [{
            "heading": "What changed",
            "fact_ids": ["metric.rate_change", "trend.daily"],
            "commentary": "The verified movement merits investigation."
        }],
        "judgment_claims": [
            {
                "claim_type": "causal",
                "text": "A temporally associated event may have contributed to the increase.",
                "support_fact_ids": ["metric.rate_change"],
                "support_memory_ids": [support_doc],
                "review_required": true
            },
            {
                "claim_type": "operational_recommendation",
                "text": "Annotate the dashboard with a non-causal warning while freshness and cause are reviewed.",
                "support_fact_ids": ["metric.rate_change", "trend.daily"],
                "support_memory_ids": [snapshot, review_policy],
                "review_required": true
            }
        ],
        "slide_outline": [{
            "title": "Metric movement this period",
            "fact_ids": ["metric.rate_change", "trend.daily"]
        }]
    }))
}

pub fn test_model() -> Arc<dyn ModelProvider> {
    Arc::new(TestModelProvider)
}

#[allow(dead_code)]
pub fn subscription_plan() -> serde_json::Value {
    plan_from_contract(&json!({
        "task": {"task_type": "subscription_churn_review"},
        "time_window": {
            "start": "2026-07-13T00:00:00Z",
            "current_start": "2026-07-20T00:00:00Z",
            "end": "2026-07-27T00:00:00Z"
        },
        "result_schemas": {
            "rate_comparison": ["period", "churned_accounts", "eligible_accounts", "churn_rate"],
            "concentration": ["plan_tier", "churn_type", "churned_accounts", "eligible_accounts", "churn_rate"],
            "timeseries": ["day", "churn_rate"]
        },
        "sql_contract": {
            "relation": "subscription_events",
            "time_field": "event_date",
            "required_metric_filters": [
                "segment = 'SMB'",
                "environment = 'production'",
                "is_test_account = 0"
            ],
            "required_time_bounds": {
                "rate_comparison": {
                    "lower": "event_date >= '2026-07-13'",
                    "upper": "event_date < '2026-07-27'"
                },
                "concentration": {
                    "lower": "event_date >= '2026-07-20'",
                    "upper": "event_date < '2026-07-27'"
                }
            }
        }
    }))
    .expect("subscription plan fixture")
}
