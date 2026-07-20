use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};
use serde_json::json;

use crate::{
    Result,
    domain::{
        Authority, Budgets, ConsistencyClass, MemoryObject, MemoryType, RiskClass, TaskDefinition,
        content_hash,
    },
    store::Store,
};

pub const TENANT: &str = "tenant_demo";
pub const SOURCE: &str = "warehouse_primary";
pub const WINDOW_START: &str = "2026-07-07T08:00:00Z";
pub const SPIKE_START: &str = "2026-07-07T14:00:00Z";
pub const WINDOW_END: &str = "2026-07-07T20:00:00Z";

pub fn seed_demo(store: &Store, warehouse_path: &Path) -> Result<()> {
    seed_warehouse(warehouse_path)?;
    let start = parse(WINDOW_START)?;
    let end = parse(WINDOW_END)?;
    let common = BTreeSet::from(["analytics".into(), "payments".into()]);
    let objects = vec![
        memory(
            "metric:payment_failure_rate",
            MemoryType::SemanticDefinition,
            "Approved payment failure rate: failed production attempts divided by production attempts, excluding test accounts.",
            json!({"role":"metric_definition","name":"payment_failure_rate","version":"v3","required_filters":["environment = 'production'","is_test_account = 0"],"owner":"payments_analytics"}),
            "semantic_layer",
            "v3",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "schema:payment_events",
            MemoryType::Schema,
            "Current payment_events schema with error_code and event-time fields.",
            json!({"role":"active_schema","table":"payment_events","version":"v2","columns":["event_id","event_time","processor","card_network","environment","is_test_account","status","error_code"],"renamed_columns":{"failure_reason":"error_code"},"blocked_columns":["customer_email","payment_token","raw_payload"]}),
            SOURCE,
            "schema-v2",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "snapshot:payment_events:492",
            MemoryType::DataState,
            "Payment event snapshot for the twelve-hour comparison window.",
            json!({"role":"data_snapshot","snapshot_id":"payment_events_snapshot_492","event_time_start":WINDOW_START,"event_time_end":WINDOW_END,"watermark":"2026-07-07T19:58:30Z","freshness_warning":"watermark is 90 seconds behind the requested end time","consistency":"C2"}),
            SOURCE,
            "snapshot-492",
            Authority::SystemObserved,
            start,
            Some(end + Duration::minutes(15)),
            common.clone(),
        )?,
        memory(
            "policy:user:analyst",
            MemoryType::PermissionPolicy,
            "Payment analysis access policy for analysts.",
            json!({"role":"user_policy","policy_epoch":1,"allowed_tools":["sql.readonly.v1","stats.rate_comparison.v1","chart.timeseries.v1"]}),
            "iam",
            "epoch-1",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "policy:review:payments",
            MemoryType::ReviewPolicy,
            "Payment operational recommendations and causal claims require reviewer approval.",
            json!({"role":"review_policy","requires_review":["causal","operational_recommendation"]}),
            "policy",
            "v1",
            Authority::OwnerApproved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "document:gateway_deploy",
            MemoryType::Document,
            "Gateway 7.8.2 deployment preceded the spike and changed Processor B retry timeout handling.",
            json!({"role":"deployment_event","deployed_at":"2026-07-07T13:35:00Z","service":"payment-gateway"}),
            "deployments",
            "2026-07-07",
            Authority::SystemObserved,
            start,
            None,
            common.clone(),
        )?,
        memory(
            "analysis:processor_b_retry",
            MemoryType::PriorAnalysis,
            "Prior reviewed incident associated Processor B retry amplification with Visa failures.",
            json!({"role":"prior_incident","review_state":"approved"}),
            "incident_review",
            "2026-06-20",
            Authority::ReviewerApproved,
            start,
            None,
            BTreeSet::from(["analytics".into(), "payments".into(), "sre".into()]),
        )?,
    ];
    for object in objects {
        store.write_memory(&object)?;
    }
    let definition = TaskDefinition {
        tenant_id: TENANT.into(),
        task_type: "payment_health_review".into(),
        version: 3,
        status: "approved".into(),
        risk_class: RiskClass::MaterialInternal,
        required_roles: BTreeMap::from([
            ("metric_definition".into(), MemoryType::SemanticDefinition),
            ("active_schema".into(), MemoryType::Schema),
            ("data_snapshot".into(), MemoryType::DataState),
            ("user_policy".into(), MemoryType::PermissionPolicy),
            ("review_policy".into(), MemoryType::ReviewPolicy),
        ]),
        optional_roles: BTreeMap::from([
            ("prior_incident".into(), MemoryType::PriorAnalysis),
            ("deployment_event".into(), MemoryType::Document),
            ("reviewer_feedback".into(), MemoryType::Feedback),
        ]),
        minimum_consistency: BTreeMap::from([
            ("metric_definition".into(), ConsistencyClass::C1),
            ("active_schema".into(), ConsistencyClass::C1),
            ("data_snapshot".into(), ConsistencyClass::C2),
            ("user_policy".into(), ConsistencyClass::C1),
        ]),
        allowed_tools: BTreeSet::from([
            "sql.readonly.v1".into(),
            "stats.rate_comparison.v1".into(),
            "chart.timeseries.v1".into(),
        ]),
        claim_types: BTreeSet::from([
            "metric_value".into(),
            "metric_comparison".into(),
            "concentration".into(),
            "causal".into(),
            "operational_recommendation".into(),
        ]),
        verifier_profile: "payment_health.v2".into(),
        publication_policy: "human_review_required".into(),
        budgets: Budgets::default(),
        artifact_schema: "payment_health_report.v3".into(),
        effective_start: Some(start),
        effective_end: None,
    };
    store.put_task_definition(&definition)?;
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
    let mut connection = Connection::open(path)?;
    connection.execute_batch("CREATE TABLE payment_events(event_id TEXT PRIMARY KEY,event_time TEXT NOT NULL,processor TEXT NOT NULL,card_network TEXT NOT NULL,environment TEXT NOT NULL,is_test_account INTEGER NOT NULL,status TEXT NOT NULL,error_code TEXT);")?;
    let tx = connection.transaction()?;
    let mut rng = 42_u64;
    let start = parse(WINDOW_START)?;
    let spike_start = parse(SPIKE_START)?;
    let mut offset = 0_u64;
    for minute in 0..720 {
        let time = start + Duration::minutes(minute);
        for _ in 0..20 {
            rng = lcg(rng);
            let processor = match rng % 100 {
                0..=41 => "Processor A",
                42..=75 => "Processor B",
                _ => "Processor C",
            };
            rng = lcg(rng);
            let network = match rng % 100 {
                0..=51 => "Visa",
                52..=87 => "Mastercard",
                _ => "Amex",
            };
            rng = lcg(rng);
            let test = rng.is_multiple_of(47);
            let environment = if rng.is_multiple_of(40) {
                "test"
            } else {
                "production"
            };
            let mut rate = 20_u64;
            if time >= spike_start {
                rate = 38;
                if processor == "Processor B" && network == "Visa" {
                    rate = 158
                } else if processor == "Processor B" {
                    rate = 80
                } else if network == "Visa" {
                    rate = 55
                }
            }
            rng = lcg(rng);
            let failed = rng % 1000 < rate;
            tx.execute(
                "INSERT INTO payment_events VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    format!("evt_{offset}"),
                    time.to_rfc3339(),
                    processor,
                    network,
                    environment,
                    test as i64,
                    if failed { "failure" } else { "success" },
                    if failed {
                        Some("processor_timeout")
                    } else {
                        None
                    }
                ],
            )?;
            offset += 1;
        }
    }
    tx.commit()?;
    Ok(())
}
fn lcg(value: u64) -> u64 {
    value.wrapping_mul(6364136223846793005).wrapping_add(1)
}
fn parse(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .map_err(|error| {
            crate::error::AmosError::Validation(format!("invalid demo fixture timestamp: {error}"))
        })?
        .with_timezone(&Utc))
}
