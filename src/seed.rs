use std::{collections::BTreeSet, path::Path};

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::json;

use crate::{
    Result,
    domain::{Authority, ConsistencyClass, MemoryObject, MemoryStatus, MemoryType, content_hash},
    packs::{
        AnalysisPack, SUBSCRIPTION_CURRENT_START, SUBSCRIPTION_SOURCE, SUBSCRIPTION_WINDOW_END,
        SUBSCRIPTION_WINDOW_START,
    },
    store::Store,
};

pub const TENANT: &str = "tenant_demo";
pub const RESTRICTED_MEMORY_CANARY: &str = "RESTRICTED_MEMORY_CANARY_4e83b66d";
pub const WAREHOUSE_RAW_CANARY: &str = "WAREHOUSE_RAW_CANARY_9f1c2e7b";

pub fn seed_demo(store: &Store, warehouse_path: &Path) -> Result<()> {
    seed_warehouse(warehouse_path)?;
    seed_subscription_memory(store)?;
    seed_payment_failure_memory(store)?;
    install_demo_packs(store)?;
    Ok(())
}

fn install_demo_packs(store: &Store) -> Result<()> {
    let installed_at = Utc::now();
    for pack in [
        AnalysisPack::subscription_churn()?,
        load_demo_pack("payment_failure_churn")?,
    ] {
        store.install_analysis_pack(TENANT, &pack, "amos:seed", installed_at)?;
    }
    Ok(())
}

fn load_demo_pack(pack_id: &str) -> Result<AnalysisPack> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("demo")
        .join(pack_id)
        .join("pack.json");
    AnalysisPack::load(path)
}

fn seed_subscription_memory(store: &Store) -> Result<()> {
    let start = parse(SUBSCRIPTION_WINDOW_START)?;
    let end = parse(SUBSCRIPTION_WINDOW_END)?;
    let common = BTreeSet::from(["analytics".into(), "subscriptions".into()]);
    let mut superseded = memory(
        "metric:logo_churn",
        MemoryType::SemanticDefinition,
        "Superseded logo churn definition retained for audit history.",
        json!({
            "role":"metric_definition",
            "name":"logo_churn",
            "version":"v3",
            "required_filters":["segment = 'SMB'","environment = 'production'"]
        }),
        "semantic_layer",
        "v3",
        Authority::OwnerApproved,
        start - Duration::days(30),
        Some(start - Duration::seconds(1)),
        common.clone(),
    )?;
    superseded.status = MemoryStatus::Superseded;

    let objects = vec![
        superseded,
        memory(
            "metric:logo_churn",
            MemoryType::SemanticDefinition,
            "Approved logo churn definition: churned eligible company accounts divided by eligible company accounts in the governed period.",
            json!({
                "role":"metric_definition",
                "name":"logo_churn",
                "version":"v4",
                "unit":"account",
                "numerator":"distinct eligible accounts with churned = 1",
                "denominator":"distinct eligible accounts",
                "required_filters":[
                    "segment = 'SMB'",
                    "environment = 'production'",
                    "is_test_account = 0"
                ],
                "owner":"subscription_analytics"
            }),
            "semantic_layer",
            "v4",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "schema:subscription_events",
            MemoryType::Schema,
            "Current subscription_events v3 schema exposes governed analytical fields and marks direct identifiers and raw notes as blocked.",
            json!({
                "role":"active_schema",
                "table":"subscription_events",
                "relation":"subscription_events",
                "time_field":"event_date",
                "version":"v3",
                "columns":[
                    "event_date","account_id","segment","plan_tier","environment",
                    "is_test_account","churned","churn_type","churn_reason",
                    "monthly_recurring_revenue","support_contact_count"
                ],
                "blocked_columns":["customer_email","raw_support_note"],
                "renamed_columns":{"cancellation_type":"churn_type"},
                "permission_labels":["analytics","subscriptions"]
            }),
            SUBSCRIPTION_SOURCE,
            "schema-v3",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "snapshot:subscription_events:2026-07-27",
            MemoryType::DataState,
            "Current subscription snapshot covers the two governed seven-day periods; the final day is incomplete.",
            json!({
                "role":"data_snapshot",
                "snapshot_id":"subscription_events_snapshot_2026_07_27",
                "relation":"subscription_events",
                "event_date_start":SUBSCRIPTION_WINDOW_START,
                "current_period_start":SUBSCRIPTION_CURRENT_START,
                "event_date_end":SUBSCRIPTION_WINDOW_END,
                "watermark":"2026-07-26T18:00:00Z",
                "freshness_warning":"the final day is incomplete and may be delayed",
                "consistency":"C2"
            }),
            SUBSCRIPTION_SOURCE,
            "subscription-snapshot-v3",
            Authority::SystemObserved,
            start,
            Some(end + Duration::days(1)),
            common.clone(),
        )?,
        memory(
            "policy:user:subscription_analyst",
            MemoryType::PermissionPolicy,
            "Analysts may run aggregate SMB logo churn and subscription analysis over governed fields only.",
            json!({
                "role":"user_policy",
                "policy_epoch":1,
                "permission_labels":["analytics","subscriptions"],
                "allowed_tools":[
                    "sql.readonly.v1",
                    "stats.rate_comparison.v1",
                    "chart.timeseries.v1"
                ],
                "aggregate_only":true
            }),
            "iam",
            "subscription-epoch-1",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "policy:review:subscriptions",
            MemoryType::ReviewPolicy,
            "Causal attributions and dashboard recommendations require separate reviewer approval.",
            json!({
                "role":"review_policy",
                "requires_review":["causal","operational_recommendation"],
                "publisher_role":"reviewer"
            }),
            "policy",
            "subscription-v1",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "document:pricing_email_launch",
            MemoryType::Document,
            "The SMB pricing email launched before the observed churn increase; timing alone is not causal evidence.",
            json!({
                "role":"launch_event",
                "campaign":"SMB pricing update",
                "launched_at":"2026-07-19T16:00:00Z",
                "causal_status":"unproven"
            }),
            "campaign_registry",
            "pricing-email-2026-07-19",
            Authority::SystemObserved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "analysis:causal_over_attribution",
            MemoryType::PriorAnalysis,
            "Prior reviewed guidance requires controlled evidence before attributing churn movement to a campaign.",
            json!({
                "role":"prior_guidance",
                "review_state":"approved",
                "guidance":"Treat temporal association as a hypothesis, not a causal conclusion."
            }),
            "analysis_review",
            "causal-guidance-v2",
            Authority::ReviewerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "analysis:churn_pipeline_incident",
            MemoryType::PriorAnalysis,
            "SRE-only review of the churn ingest pipeline incident; not visible to subscription analysts.",
            json!({
                "role":"incident_review",
                "review_state":"approved",
                "guidance":"Ingest delays can understate the final day of churn; confirm watermarks before publication."
            }),
            "sre_review",
            "churn-ingest-incident-v1",
            Authority::ReviewerApproved,
            start,
            None,
            BTreeSet::from(["analytics".into(), "sre".into()]),
        )?,
        memory(
            "incident:restricted_raw_support_note",
            MemoryType::Document,
            "Restricted raw support incident unavailable to subscription analysts.",
            json!({
                "role":"restricted_incident",
                "raw_note":RESTRICTED_MEMORY_CANARY
            }),
            "support_system",
            "restricted-incident-v1",
            Authority::SystemObserved,
            start,
            None,
            BTreeSet::from(["analytics".into(), "restricted_support".into()]),
        )?,
    ];
    for object in objects {
        store.write_memory(&object)?;
    }
    Ok(())
}

fn seed_payment_failure_memory(store: &Store) -> Result<()> {
    let start = parse(SUBSCRIPTION_WINDOW_START)?;
    let common = BTreeSet::from(["analytics".into(), "subscriptions".into()]);
    store.write_memory(&memory(
        "metric:payment_failure_churn",
        MemoryType::SemanticDefinition,
        "Approved payment-failure churn definition: eligible accounts with churn_reason payment_failure divided by eligible accounts in the governed period.",
        json!({
            "role":"metric_definition",
            "name":"payment_failure_churn",
            "version":"v1",
            "unit":"account",
            "numerator":"distinct eligible accounts with churn_reason = payment_failure and churned = 1",
            "denominator":"distinct eligible accounts",
            "required_filters":[
                "segment = 'SMB'",
                "environment = 'production'",
                "is_test_account = 0"
            ],
            "owner":"revenue_operations"
        }),
        "semantic_layer",
        "v1",
        Authority::OwnerApproved,
        start,
        None,
        common,
    )?)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn memory(
    key: &str,
    memory_type: MemoryType,
    summary: &str,
    content: serde_json::Value,
    source: &str,
    version: &str,
    authority: Authority,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    permissions: BTreeSet<String>,
) -> Result<MemoryObject> {
    let mut object = MemoryObject::new(
        TENANT,
        key,
        memory_type,
        summary,
        content,
        source,
        version,
        authority,
    )?;
    object.effective_start = Some(start);
    object.effective_end = end;
    object.permissions = permissions;
    object.version = version.into();
    object.consistency_class = match memory_type {
        MemoryType::DataState | MemoryType::StreamState
            if object
                .content
                .get("consistency")
                .and_then(|value| value.as_str())
                == Some("C2") =>
        {
            ConsistencyClass::C2
        }
        MemoryType::SemanticDefinition
        | MemoryType::Schema
        | MemoryType::PermissionPolicy
        | MemoryType::ReviewPolicy => ConsistencyClass::C1,
        _ => ConsistencyClass::C0,
    };
    object.content_hash = content_hash(&object.content)?;
    Ok(object)
}

pub fn seed_warehouse(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::error::AmosError::Storage(e.to_string()))?;
    }
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| crate::error::AmosError::Storage(e.to_string()))?;
    }
    let connection = Connection::open(path)?;
    connection.execute_batch(include_str!("../demo/subscription_churn/warehouse.sql"))?;
    Ok(())
}
fn parse(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            crate::error::AmosError::Validation(format!("invalid demo fixture timestamp: {error}"))
        })?
        .with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        RuntimeConfig,
        auth::demo_identities,
        context::ContextCompiler,
        memory::MemoryService,
        packs::{SUBSCRIPTION_TASK_TYPE, SUBSCRIPTION_WINDOW_END, SUBSCRIPTION_WINDOW_START},
        policy::PolicyEngine,
    };
    use tempfile::TempDir;

    #[test]
    fn subscription_fixture_is_reproducible_and_context_excludes_both_canaries() {
        let root = TempDir::new().expect("temp root");
        let config = RuntimeConfig::demo(root.path()).expect("subscription config");
        assert_eq!(config.analysis_pack.task_type, SUBSCRIPTION_TASK_TYPE);
        let store = Store::open(&config.control_db).expect("store");
        seed_demo(&store, &config.warehouse_db).expect("seed");

        let warehouse = Connection::open(&config.warehouse_db).expect("warehouse");
        let rates = warehouse
            .prepare(
                "SELECT
                    CASE WHEN event_date < '2026-07-20' THEN 'baseline' ELSE 'current' END,
                    SUM(churned),
                    COUNT(*)
                 FROM subscription_events
                 WHERE segment='SMB' AND environment='production' AND is_test_account=0
                 GROUP BY 1 ORDER BY 1",
            )
            .expect("rate query")
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .expect("rate rows")
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("rates");
        assert_eq!(
            rates,
            vec![
                ("baseline".into(), 62, 2_000),
                ("current".into(), 108, 2_000)
            ]
        );
        let raw_canary: String = warehouse
            .query_row(
                "SELECT raw_support_note FROM subscription_events
                 WHERE raw_support_note=?1 LIMIT 1",
                [WAREHOUSE_RAW_CANARY],
                |row| row.get(0),
            )
            .expect("warehouse canary");
        assert_eq!(raw_canary, WAREHOUSE_RAW_CANARY);

        let definition = store
            .get_task_definition(TENANT, SUBSCRIPTION_TASK_TYPE)
            .expect("task definition")
            .expect("subscription task");
        let identity = demo_identities().remove("analyst_001").expect("analyst");
        let compiler = ContextCompiler::new(MemoryService::new(store.clone(), PolicyEngine));
        let manifest = compiler
            .compile(
                &identity,
                "atxn_subscription_context",
                "Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
                &definition,
                parse(SUBSCRIPTION_WINDOW_START).expect("start"),
                parse(SUBSCRIPTION_WINDOW_END).expect("end"),
            )
            .expect("context manifest");
        let selected = manifest
            .selected_objects
            .iter()
            .map(|object| (object.logical_key.as_str(), object.source_version.as_str()))
            .collect::<BTreeSet<_>>();
        assert!(selected.contains(&("metric:logo_churn", "v4")));
        assert!(selected.contains(&("schema:subscription_events", "schema-v3")));
        assert!(selected.contains(&(
            "snapshot:subscription_events:2026-07-27",
            "subscription-snapshot-v3"
        )));
        assert!(
            !manifest
                .selected_objects
                .iter()
                .any(|object| object.logical_key == "incident:restricted_raw_support_note")
        );

        let serialized_payload = serde_json::to_string(&json!({
            "question":"Why did SMB logo churn increase?",
            "selected_governed_objects":manifest.selected_objects,
            "verified_aggregate_results":[]
        }))
        .expect("model payload");
        assert!(!serialized_payload.contains(RESTRICTED_MEMORY_CANARY));
        assert!(!serialized_payload.contains(WAREHOUSE_RAW_CANARY));
        assert!(serialized_payload.contains("raw_support_note"));
    }
}
