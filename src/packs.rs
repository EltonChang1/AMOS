use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AmosError, Result,
    domain::{Budgets, ConsistencyClass, MemoryType, RiskClass, TaskDefinition},
    model::AnalysisKind,
};

pub const SUBSCRIPTION_PACK_ID: &str = "subscription_churn";
pub const SUBSCRIPTION_TASK_TYPE: &str = "subscription_churn_review";
pub const SUBSCRIPTION_RELATION: &str = "subscription_events";
pub const SUBSCRIPTION_SOURCE: &str = "warehouse_primary";
pub const SUBSCRIPTION_WINDOW_START: &str = "2026-07-13T00:00:00Z";
pub const SUBSCRIPTION_CURRENT_START: &str = "2026-07-20T00:00:00Z";
pub const SUBSCRIPTION_WINDOW_END: &str = "2026-07-27T00:00:00Z";

pub const PAYMENT_FAILURE_PACK_ID: &str = "payment_failure_churn";
pub const PAYMENT_FAILURE_TASK_TYPE: &str = "payment_failure_churn_review";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPack {
    pub pack_id: String,
    pub task_type: String,
    pub version: u32,
    pub status: String,
    pub risk_class: RiskClass,
    pub time_window: PackTimeWindow,
    pub required_roles: BTreeMap<String, MemoryType>,
    pub optional_roles: BTreeMap<String, MemoryType>,
    pub minimum_consistency: BTreeMap<String, ConsistencyClass>,
    pub allowed_tools: BTreeSet<String>,
    pub budgets: Budgets,
    pub source_relations: BTreeMap<String, String>,
    pub schemas: Vec<PackRelationSchema>,
    pub metric_required_filters: Vec<String>,
    pub required_analysis_kinds: BTreeSet<AnalysisKind>,
    pub result_schemas: BTreeMap<AnalysisKind, Vec<String>>,
    pub verifier_profile: GenericVerifierProfile,
    pub review_triggering_claim_types: BTreeSet<String>,
    pub claim_types: BTreeSet<String>,
    pub chart: ChartPackConfig,
    pub report_template: String,
    pub artifact_schema: String,
    pub audience: String,
    pub publication_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackTimeWindow {
    pub start: String,
    pub current_start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackRelationSchema {
    pub relation: String,
    pub time_field: String,
    pub permission_labels: BTreeSet<String>,
    pub allowed_columns: BTreeSet<String>,
    pub blocked_columns: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GenericVerifierProfile {
    pub profile_id: String,
    pub rate_comparison: RateComparisonFields,
    pub concentration: RateFields,
    pub timeseries: TimeseriesFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RateComparisonFields {
    pub period_field: String,
    pub current_label: String,
    pub baseline_label: String,
    pub numerator_field: String,
    pub denominator_field: String,
    pub rate_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RateFields {
    pub numerator_field: String,
    pub denominator_field: String,
    pub rate_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimeseriesFields {
    pub label_field: String,
    pub value_field: String,
    pub accessible_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChartPackConfig {
    pub title: String,
    pub x_label: String,
    pub y_label: String,
}

impl AnalysisPack {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_json_slice(&bytes)
    }

    pub fn subscription_churn() -> Result<Self> {
        Self::from_json_slice(include_bytes!("../demo/subscription_churn/pack.json"))
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let document: Value = serde_json::from_slice(bytes)?;
        let schema: Value = serde_json::from_str(include_str!("../demo/pack.schema.json"))?;
        let validator = jsonschema::validator_for(&schema)
            .map_err(|_| AmosError::Validation("analysis pack schema is invalid".into()))?;
        if let Err(error) = validator.validate(&document) {
            return Err(AmosError::Validation(format!(
                "analysis pack JSON schema validation failed at {}",
                error.instance_path()
            )));
        }
        let pack: Self = serde_json::from_value(document)?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn validate(&self) -> Result<()> {
        if self.pack_id.trim().is_empty()
            || self.task_type.trim().is_empty()
            || self.version == 0
            || self.status != "approved"
            || self.schemas.is_empty()
            || self.source_relations.is_empty()
        {
            return Err(AmosError::Validation(
                "analysis pack identity and source fields are incomplete".into(),
            ));
        }
        let start = parse_time(&self.time_window.start)?;
        let current = parse_time(&self.time_window.current_start)?;
        let end = parse_time(&self.time_window.end)?;
        if !(start < current && current < end) {
            return Err(AmosError::Validation(
                "analysis pack time bounds must be strictly ordered".into(),
            ));
        }
        for schema in &self.schemas {
            if schema.relation.trim().is_empty()
                || schema.time_field.trim().is_empty()
                || schema.allowed_columns.is_empty()
                || schema.permission_labels.is_empty()
                || !schema.allowed_columns.is_disjoint(&schema.blocked_columns)
                || !self.source_relations.contains_key(&schema.relation)
            {
                return Err(AmosError::Validation(
                    "analysis pack relation schema is inconsistent".into(),
                ));
            }
        }
        let expected_kinds = BTreeSet::from([
            AnalysisKind::RateComparison,
            AnalysisKind::Concentration,
            AnalysisKind::Timeseries,
        ]);
        if self.required_analysis_kinds != expected_kinds
            || self.result_schemas.keys().copied().collect::<BTreeSet<_>>() != expected_kinds
            || self.result_schemas.values().any(Vec::is_empty)
        {
            return Err(AmosError::Validation(
                "analysis pack must define all required result schemas".into(),
            ));
        }
        if self.metric_required_filters.is_empty()
            || self.report_template.trim().is_empty()
            || self.artifact_schema.trim().is_empty()
            || self.audience.trim().is_empty()
            || self.publication_policy.trim().is_empty()
        {
            return Err(AmosError::Validation(
                "analysis pack governance and output fields are incomplete".into(),
            ));
        }
        Ok(())
    }

    pub fn start(&self) -> Result<DateTime<Utc>> {
        parse_time(&self.time_window.start)
    }

    pub fn current_start(&self) -> Result<DateTime<Utc>> {
        parse_time(&self.time_window.current_start)
    }

    pub fn end(&self) -> Result<DateTime<Utc>> {
        parse_time(&self.time_window.end)
    }

    pub fn primary_relation(&self) -> Result<(&str, &str)> {
        if self.source_relations.len() != 1 {
            return Err(AmosError::Validation(
                "this vertical slice requires exactly one configured source relation".into(),
            ));
        }
        self.source_relations
            .iter()
            .next()
            .map(|(relation, source)| (relation.as_str(), source.as_str()))
            .ok_or_else(|| AmosError::Validation("analysis pack has no source relation".into()))
    }

    pub fn to_task_definition(&self, tenant_id: &str) -> Result<TaskDefinition> {
        self.validate()?;
        Ok(TaskDefinition {
            tenant_id: tenant_id.into(),
            task_type: self.task_type.clone(),
            version: self.version,
            status: self.status.clone(),
            risk_class: self.risk_class,
            required_roles: self.required_roles.clone(),
            optional_roles: self.optional_roles.clone(),
            minimum_consistency: self.minimum_consistency.clone(),
            allowed_tools: self.allowed_tools.clone(),
            claim_types: self.claim_types.clone(),
            verifier_profile: self.verifier_profile.profile_id.clone(),
            publication_policy: self.publication_policy.clone(),
            budgets: self.budgets.clone(),
            artifact_schema: self.artifact_schema.clone(),
            effective_start: Some(self.start()?),
            effective_end: None,
        })
    }
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .map_err(|_| AmosError::Validation("analysis pack timestamp is invalid".into()))?
        .with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_pack_passes_json_schema_and_semantic_validation() {
        let pack = AnalysisPack::subscription_churn().expect("subscription pack");
        assert_eq!(pack.pack_id, SUBSCRIPTION_PACK_ID);
        assert_eq!(
            pack.source_relations.get(SUBSCRIPTION_RELATION),
            Some(&SUBSCRIPTION_SOURCE.to_string())
        );
        assert!(pack.schemas[0].blocked_columns.contains("raw_support_note"));
    }

    #[test]
    fn pack_loader_rejects_unknown_fields_and_incomplete_kinds() {
        let mut document: Value =
            serde_json::from_slice(include_bytes!("../demo/subscription_churn/pack.json"))
                .expect("pack JSON");
        document
            .as_object_mut()
            .expect("pack object")
            .insert("warehouse_credentials".into(), Value::Null);
        let bytes = serde_json::to_vec(&document).expect("modified pack");
        assert!(AnalysisPack::from_json_slice(&bytes).is_err());
    }
}
