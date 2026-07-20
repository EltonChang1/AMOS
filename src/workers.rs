use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Map, Value, json};
use sha2::Sha256;

use crate::{
    Result,
    domain::{
        CapabilityClaims, CapabilityEnvelope, CapabilityLimits, ExecutionRecord, Identity,
        PlanStep, TypedPlan, content_hash, new_id,
    },
    error::AmosError,
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct CapabilityIssuer {
    secret: Vec<u8>,
    issuer: String,
}

impl CapabilityIssuer {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self> {
        let secret = secret.as_ref();
        if secret.len() < 32 {
            return Err(AmosError::Capability(
                "capability signing key must be at least 32 bytes".into(),
            ));
        }
        Ok(Self {
            secret: secret.to_vec(),
            issuer: "amos-runtime".into(),
        })
    }
    pub fn issue(
        &self,
        identity: &Identity,
        plan: &TypedPlan,
        step: &PlanStep,
        fencing_token: u64,
    ) -> Result<CapabilityEnvelope> {
        let now = Utc::now().timestamp();
        let relations = declared_relations(step)?;
        let claims = CapabilityClaims {
            issuer: self.issuer.clone(),
            audience: audience(&step.tool),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: plan.atxn_id.clone(),
            plan_id: plan.plan_id.clone(),
            step_id: step.step_id.clone(),
            subject_id: identity.subject_id.clone(),
            tool: step.tool.clone(),
            source_id: step.source_id.clone(),
            operations: BTreeSet::from(["query".into()]),
            relations,
            limits: CapabilityLimits {
                seconds: step.limits.seconds,
                rows: step.limits.rows,
                bytes: step.limits.bytes,
            },
            policy_epoch: identity.policy_epoch,
            fencing_token,
            token_id: new_id("cap"),
            not_before: now - 1,
            expires_at: now + 60,
        };
        let signature = self.sign(&claims)?;
        Ok(CapabilityEnvelope { claims, signature })
    }
    pub fn validate(
        &self,
        envelope: &CapabilityEnvelope,
        audience: &str,
        policy_epoch: u64,
        fence: u64,
    ) -> Result<()> {
        let signature = URL_SAFE_NO_PAD
            .decode(&envelope.signature)
            .map_err(|_| AmosError::Capability("invalid signature encoding".into()))?;
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|error| AmosError::Capability(error.to_string()))?;
        mac.update(&serde_json::to_vec(&envelope.claims)?);
        mac.verify_slice(&signature)
            .map_err(|_| AmosError::Capability("invalid signature".into()))?;
        let now = Utc::now().timestamp();
        if envelope.claims.audience != audience || envelope.claims.issuer != self.issuer {
            return Err(AmosError::Capability("issuer or audience mismatch".into()));
        }
        if envelope.claims.token_id.trim().is_empty()
            || envelope.claims.expires_at <= envelope.claims.not_before
            || envelope.claims.expires_at - envelope.claims.not_before > 120
        {
            return Err(AmosError::Capability(
                "invalid token identifier or validity window".into(),
            ));
        }
        if now < envelope.claims.not_before || now >= envelope.claims.expires_at {
            return Err(AmosError::Capability(
                "capability expired or not active".into(),
            ));
        }
        if envelope.claims.policy_epoch != policy_epoch || envelope.claims.fencing_token != fence {
            return Err(AmosError::Capability(
                "policy epoch or fence mismatch".into(),
            ));
        }
        Ok(())
    }
    fn sign(&self, claims: &CapabilityClaims) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|e| AmosError::Capability(e.to_string()))?;
        mac.update(&serde_json::to_vec(claims)?);
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }
}

#[derive(Clone)]
pub struct SqlWorker {
    path: PathBuf,
    issuer: CapabilityIssuer,
}

#[derive(Clone, Default)]
pub struct ExecutionCancellation {
    cancelled: Arc<AtomicBool>,
}

impl ExecutionCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl SqlWorker {
    pub fn new(path: impl AsRef<Path>, issuer: CapabilityIssuer) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            issuer,
        }
    }
    pub fn execute(
        &self,
        identity: &Identity,
        plan: &TypedPlan,
        step: &PlanStep,
        capability: &CapabilityEnvelope,
        fence: u64,
    ) -> Result<ExecutionRecord> {
        self.execute_with_cancellation(
            identity,
            plan,
            step,
            capability,
            fence,
            &ExecutionCancellation::default(),
        )
    }

    pub fn execute_with_cancellation(
        &self,
        identity: &Identity,
        plan: &TypedPlan,
        step: &PlanStep,
        capability: &CapabilityEnvelope,
        fence: u64,
        cancellation: &ExecutionCancellation,
    ) -> Result<ExecutionRecord> {
        self.issuer
            .validate(capability, "sql-worker", identity.policy_epoch, fence)?;
        validate_sql_bindings(identity, plan, step, capability, fence)?;
        let sql = step
            .parameters
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AmosError::Validation("SQL step has no sql parameter".into()))?;
        let started = Instant::now();
        let connection =
            Connection::open(&self.path).map_err(|e| AmosError::Execution(e.to_string()))?;
        connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
        let deadline = started + Duration::from_secs(step.limits.seconds);
        let cancelled = cancellation.cancelled.clone();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timeout_observed = timed_out.clone();
        connection.progress_handler(
            1_000,
            Some(move || {
                if cancelled.load(Ordering::Acquire) {
                    return true;
                }
                if Instant::now() >= deadline {
                    timeout_observed.store(true, Ordering::Release);
                    return true;
                }
                false
            }),
        )?;
        let mut statement = connection
            .prepare(sql)
            .map_err(|e| AmosError::Execution(e.to_string()))?;
        let names: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut rows = statement.query([]).map_err(|error| {
            if cancellation.is_cancelled() {
                AmosError::Execution("query cancelled".into())
            } else if timed_out.load(Ordering::Acquire) || Instant::now() >= deadline {
                AmosError::Execution("query timeout exceeded".into())
            } else {
                AmosError::Execution(error.to_string())
            }
        })?;
        let mut output = vec![];
        let mut byte_count = 2_u64;
        if byte_count > step.limits.bytes {
            return Err(AmosError::Execution("byte limit exceeded".into()));
        }
        loop {
            let next = rows.next().map_err(|error| {
                if cancellation.is_cancelled() {
                    AmosError::Execution("query cancelled".into())
                } else if timed_out.load(Ordering::Acquire) || Instant::now() >= deadline {
                    AmosError::Execution("query timeout exceeded".into())
                } else {
                    AmosError::Execution(error.to_string())
                }
            })?;
            let Some(row) = next else {
                break;
            };
            if output.len() as u64 >= step.limits.rows {
                return Err(AmosError::Execution("row limit exceeded".into()));
            }
            let mut object = Map::new();
            for (index, name) in names.iter().enumerate() {
                object.insert(name.clone(), sql_value(row.get_ref(index)?)?);
            }
            let serialized = serde_json::to_vec(&object)?;
            let separator = u64::from(!output.is_empty());
            byte_count = byte_count
                .checked_add(separator)
                .and_then(|count| count.checked_add(serialized.len() as u64))
                .ok_or_else(|| AmosError::Execution("byte accounting overflow".into()))?;
            if byte_count > step.limits.bytes {
                return Err(AmosError::Execution("byte limit exceeded".into()));
            }
            output.push(Value::Object(object));
        }
        let value = Value::Array(output);
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() as u64 != byte_count {
            return Err(AmosError::Execution(
                "incremental byte accounting mismatch".into(),
            ));
        }
        Ok(ExecutionRecord {
            execution_id: new_id("exec"),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: plan.atxn_id.clone(),
            plan_id: plan.plan_id.clone(),
            step_id: step.step_id.clone(),
            tool: step.tool.clone(),
            tool_version: "sql.readonly.v1".into(),
            capability_id: capability.claims.token_id.clone(),
            parameters: step.parameters.clone(),
            parameters_hash: content_hash(&step.parameters)?,
            input_versions: BTreeMap::new(),
            output: value.clone(),
            output_hash: content_hash(&value)?,
            row_count: value.as_array().map_or(0, |v| v.len() as u64),
            byte_count: bytes.len() as u64,
            latency_ms: started.elapsed().as_millis() as u64,
            fencing_token: fence,
            status: "pass".into(),
            created_at: Utc::now(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct StatisticsWorker;
impl StatisticsWorker {
    pub fn rate_comparison(
        &self,
        current_failures: u64,
        current_total: u64,
        baseline_failures: u64,
        baseline_total: u64,
    ) -> Result<Value> {
        if current_total == 0 || baseline_total == 0 {
            return Err(AmosError::Validation(
                "rate denominator must be positive".into(),
            ));
        }
        Ok(
            json!({"current_rate":current_failures as f64/current_total as f64,"baseline_rate":baseline_failures as f64/baseline_total as f64,"absolute_change":current_failures as f64/current_total as f64-baseline_failures as f64/baseline_total as f64}),
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChartWorker;
impl ChartWorker {
    pub fn timeseries_svg(&self, points: &[(String, f64)]) -> Result<(String, String)> {
        if points.is_empty() {
            return Err(AmosError::Validation("chart requires points".into()));
        }
        let max = points
            .iter()
            .map(|(_, v)| *v)
            .fold(0.0_f64, f64::max)
            .max(0.01);
        let coords = points
            .iter()
            .enumerate()
            .map(|(i, (_, v))| {
                let x = 40.0 + i as f64 * (520.0 / (points.len().saturating_sub(1).max(1) as f64));
                let y = 220.0 - (v / max) * 180.0;
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 260" role="img" aria-label="Failure rate by hour"><rect width="600" height="260" fill="#fffefa"/><line x1="40" y1="220" x2="560" y2="220" stroke="#b7beb8"/><polyline fill="none" stroke="#1f5a3d" stroke-width="4" points="{coords}"/></svg>"##
        );
        let hash = content_hash(&svg)?;
        Ok((svg, hash))
    }
}

fn declared_relations(step: &PlanStep) -> Result<BTreeSet<String>> {
    let values = step
        .parameters
        .get("relations")
        .and_then(Value::as_array)
        .ok_or_else(|| AmosError::Validation("step must declare relations".into()))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|relation| !relation.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    AmosError::Validation(
                        "step relations must contain only non-empty strings".into(),
                    )
                })
        })
        .collect()
}

fn validate_sql_bindings(
    identity: &Identity,
    plan: &TypedPlan,
    step: &PlanStep,
    capability: &CapabilityEnvelope,
    fence: u64,
) -> Result<()> {
    let claims = &capability.claims;
    let expected_limits = CapabilityLimits {
        seconds: step.limits.seconds,
        rows: step.limits.rows,
        bytes: step.limits.bytes,
    };
    let expected_operations = BTreeSet::from(["query".to_string()]);
    let plan_step_matches = plan
        .steps
        .iter()
        .find(|candidate| candidate.step_id == step.step_id)
        == Some(step);
    if identity.tenant_id != plan.tenant_id
        || claims.tenant_id != identity.tenant_id
        || claims.atxn_id != plan.atxn_id
        || claims.plan_id != plan.plan_id
        || claims.step_id != step.step_id
        || claims.subject_id != identity.subject_id
        || claims.tool != step.tool
        || claims.source_id != step.source_id
        || claims.operations != expected_operations
        || claims.relations != declared_relations(step)?
        || claims.limits != expected_limits
        || claims.limits.seconds == 0
        || claims.limits.rows == 0
        || claims.limits.bytes == 0
        || claims.policy_epoch != identity.policy_epoch
        || claims.fencing_token != fence
        || !plan_step_matches
    {
        return Err(AmosError::Capability(
            "capability is not fully bound to the invoked plan step".into(),
        ));
    }
    Ok(())
}

fn audience(tool: &str) -> String {
    if tool.starts_with("sql.") {
        "sql-worker"
    } else if tool.starts_with("stats.") {
        "stats-worker"
    } else {
        "chart-worker"
    }
    .into()
}
fn sql_value(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => json!(v),
        ValueRef::Real(v) => json!(v),
        ValueRef::Text(v) => Value::String(
            std::str::from_utf8(v)
                .map_err(|_| AmosError::Execution("query returned invalid UTF-8 text".into()))?
                .to_owned(),
        ),
        ValueRef::Blob(v) => Value::String(URL_SAFE_NO_PAD.encode(v)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::OperationLimits;
    use std::collections::{BTreeMap, BTreeSet};
    use tempfile::NamedTempFile;
    #[test]
    fn signed_capability_rejects_tampering() {
        let issuer = CapabilityIssuer::new([7u8; 32]).unwrap();
        let identity = Identity {
            tenant_id: "t".into(),
            subject_id: "u".into(),
            roles: BTreeSet::new(),
            groups: BTreeSet::new(),
            permissions: BTreeSet::from(["payments".into()]),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 2,
        };
        let step = PlanStep {
            step_id: "s".into(),
            purpose: "p".into(),
            tool: "sql.readonly.v1".into(),
            source_id: "w".into(),
            input_object_ids: vec![],
            parameter_schema: "p".into(),
            parameters: json!({"relations":["payments"]}),
            expected_output_schema: "o".into(),
            limits: OperationLimits {
                seconds: 1,
                rows: 1,
                bytes: 100,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: "v".into(),
        };
        let plan = TypedPlan {
            plan_id: "p".into(),
            tenant_id: "t".into(),
            atxn_id: "a".into(),
            task_definition: "d".into(),
            manifest_id: "m".into(),
            model_identity: "deterministic".into(),
            steps: vec![step.clone()],
        };
        let mut cap = issuer.issue(&identity, &plan, &step, 3).unwrap();
        cap.claims.source_id = "evil".into();
        assert!(issuer.validate(&cap, "sql-worker", 2, 3).is_err());
    }

    #[test]
    fn worker_rejects_every_mismatched_capability_binding() {
        let issuer = CapabilityIssuer::new([8_u8; 32]).unwrap();
        let identity = Identity {
            tenant_id: "tenant".into(),
            subject_id: "subject".into(),
            roles: BTreeSet::new(),
            groups: BTreeSet::new(),
            permissions: BTreeSet::from(["payments".into()]),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 7,
        };
        let step = PlanStep {
            step_id: "step".into(),
            purpose: "read payments".into(),
            tool: "sql.readonly.v1".into(),
            source_id: "warehouse".into(),
            input_object_ids: vec![],
            parameter_schema: "query.v1".into(),
            parameters: json!({
                "sql": "SELECT id FROM payments",
                "relations": ["payments"]
            }),
            expected_output_schema: "rows.v1".into(),
            limits: OperationLimits {
                seconds: 1,
                rows: 10,
                bytes: 1_000,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: "test.v1".into(),
        };
        let plan = TypedPlan {
            plan_id: "plan".into(),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: "atxn".into(),
            task_definition: "task:v1".into(),
            manifest_id: "manifest".into(),
            model_identity: "deterministic".into(),
            steps: vec![step.clone()],
        };
        let database = NamedTempFile::new().unwrap();
        Connection::open(database.path())
            .unwrap()
            .execute("CREATE TABLE payments(id TEXT)", [])
            .unwrap();
        let worker = SqlWorker::new(database.path(), issuer.clone());
        let capability = issuer.issue(&identity, &plan, &step, 4).unwrap();
        worker
            .execute(&identity, &plan, &step, &capability, 4)
            .unwrap();

        type ClaimMutation = Box<dyn Fn(&mut CapabilityClaims)>;
        let mutations: Vec<ClaimMutation> = vec![
            Box::new(|claims| claims.tenant_id = "other".into()),
            Box::new(|claims| claims.atxn_id = "other".into()),
            Box::new(|claims| claims.plan_id = "other".into()),
            Box::new(|claims| claims.step_id = "other".into()),
            Box::new(|claims| claims.subject_id = "other".into()),
            Box::new(|claims| claims.tool = "chart.timeseries.v1".into()),
            Box::new(|claims| claims.source_id = "other".into()),
            Box::new(|claims| {
                claims.operations.insert("write".into());
            }),
            Box::new(|claims| {
                claims.relations.insert("secret_table".into());
            }),
            Box::new(|claims| claims.limits.rows += 1),
            Box::new(|claims| claims.policy_epoch += 1),
            Box::new(|claims| claims.fencing_token += 1),
        ];
        for mutate in mutations {
            let mut mismatched = capability.clone();
            mutate(&mut mismatched.claims);
            mismatched.signature = issuer.sign(&mismatched.claims).unwrap();
            assert!(matches!(
                worker.execute(&identity, &plan, &step, &mismatched, 4),
                Err(AmosError::Capability(_))
            ));
        }

        let mut unregistered_step = step.clone();
        unregistered_step.purpose = "different invocation".into();
        assert!(matches!(
            worker.execute(&identity, &plan, &unregistered_step, &capability, 4),
            Err(AmosError::Capability(_))
        ));
    }

    #[test]
    fn worker_enforces_incremental_bytes_timeout_and_cancellation() {
        let issuer = CapabilityIssuer::new([6_u8; 32]).unwrap();
        let identity = Identity {
            tenant_id: "tenant".into(),
            subject_id: "subject".into(),
            roles: BTreeSet::new(),
            groups: BTreeSet::new(),
            permissions: BTreeSet::from(["payments".into()]),
            policy_attributes: BTreeMap::new(),
            policy_epoch: 9,
        };
        let database = NamedTempFile::new().unwrap();
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute("CREATE TABLE payments(value TEXT)", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO payments(value) VALUES (?1)",
                ["x".repeat(10_000)],
            )
            .unwrap();
        drop(connection);
        let worker = SqlWorker::new(database.path(), issuer.clone());
        let mut step = PlanStep {
            step_id: "bounded".into(),
            purpose: "bounded query".into(),
            tool: "sql.readonly.v1".into(),
            source_id: "warehouse".into(),
            input_object_ids: vec![],
            parameter_schema: "query.v1".into(),
            parameters: json!({
                "sql": "SELECT value FROM payments",
                "relations": ["payments"]
            }),
            expected_output_schema: "rows.v1".into(),
            limits: OperationLimits {
                seconds: 5,
                rows: 10,
                bytes: 100,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: "test.v1".into(),
        };
        let mut plan = TypedPlan {
            plan_id: "plan".into(),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: "atxn".into(),
            task_definition: "task:v1".into(),
            manifest_id: "manifest".into(),
            model_identity: "deterministic".into(),
            steps: vec![step.clone()],
        };
        let capability = issuer.issue(&identity, &plan, &step, 2).unwrap();
        assert!(matches!(
            worker.execute(&identity, &plan, &step, &capability, 2),
            Err(AmosError::Execution(message)) if message == "byte limit exceeded"
        ));

        step.step_id = "timeout".into();
        step.parameters["sql"] = json!(
            "WITH RECURSIVE count(x) AS (
                VALUES(0) UNION ALL SELECT x + 1 FROM count WHERE x < 1000000000
             ) SELECT sum(x) FROM count"
        );
        step.limits = OperationLimits {
            seconds: 1,
            rows: 10,
            bytes: 1_000,
        };
        plan.steps = vec![step.clone()];
        let capability = issuer.issue(&identity, &plan, &step, 2).unwrap();
        let started = Instant::now();
        assert!(matches!(
            worker.execute(&identity, &plan, &step, &capability, 2),
            Err(AmosError::Execution(message)) if message == "query timeout exceeded"
        ));
        assert!(started.elapsed() < Duration::from_secs(3));

        step.step_id = "cancel".into();
        step.limits.seconds = 30;
        plan.steps = vec![step.clone()];
        let capability = issuer.issue(&identity, &plan, &step, 2).unwrap();
        let cancellation = ExecutionCancellation::default();
        let thread_cancellation = cancellation.clone();
        let thread_worker = worker.clone();
        let thread_identity = identity.clone();
        let thread_plan = plan.clone();
        let thread_step = step.clone();
        let thread_capability = capability.clone();
        let handle = std::thread::spawn(move || {
            thread_worker.execute_with_cancellation(
                &thread_identity,
                &thread_plan,
                &thread_step,
                &thread_capability,
                2,
                &thread_cancellation,
            )
        });
        std::thread::sleep(Duration::from_millis(10));
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
        assert!(matches!(
            handle.join().unwrap(),
            Err(AmosError::Execution(message)) if message == "query cancelled"
        ));
    }
}
