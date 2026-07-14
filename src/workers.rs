use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
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
            return Err(AmosError::Validation(
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
        let expected = self.sign(&envelope.claims)?;
        if expected != envelope.signature {
            return Err(AmosError::Capability("invalid signature".into()));
        }
        let now = Utc::now().timestamp();
        if envelope.claims.audience != audience || envelope.claims.issuer != self.issuer {
            return Err(AmosError::Capability("issuer or audience mismatch".into()));
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
        self.issuer
            .validate(capability, "sql-worker", identity.policy_epoch, fence)?;
        if capability.claims.step_id != step.step_id || capability.claims.atxn_id != plan.atxn_id {
            return Err(AmosError::Capability("plan scope mismatch".into()));
        }
        let sql = step
            .parameters
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AmosError::Validation("SQL step has no sql parameter".into()))?;
        let started = Instant::now();
        let connection =
            Connection::open(&self.path).map_err(|e| AmosError::Execution(e.to_string()))?;
        connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
        let mut statement = connection
            .prepare(sql)
            .map_err(|e| AmosError::Execution(e.to_string()))?;
        let names: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut rows = statement.query([])?;
        let mut output = vec![];
        while let Some(row) = rows.next()? {
            if output.len() as u64 >= step.limits.rows {
                return Err(AmosError::Execution("row limit exceeded".into()));
            }
            let mut object = Map::new();
            for (index, name) in names.iter().enumerate() {
                object.insert(name.clone(), sql_value(row.get_ref(index)?));
            }
            output.push(Value::Object(object));
        }
        let value = Value::Array(output);
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() as u64 > step.limits.bytes {
            return Err(AmosError::Execution("byte limit exceeded".into()));
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
            parameters_hash: content_hash(&step.parameters),
            input_versions: BTreeMap::new(),
            output: value.clone(),
            output_hash: content_hash(&value),
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
        let hash = content_hash(&svg);
        Ok((svg, hash))
    }
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
fn sql_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => json!(v),
        ValueRef::Real(v) => json!(v),
        ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).into_owned()),
        ValueRef::Blob(v) => Value::String(URL_SAFE_NO_PAD.encode(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::OperationLimits;
    use std::collections::{BTreeMap, BTreeSet};
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
}
