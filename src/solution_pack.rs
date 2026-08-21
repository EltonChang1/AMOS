use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::{
    Result,
    domain::{ConsistencyClass, MemoryType, OperationLimits, Outcome, RiskClass, content_hash},
    error::AmosError,
};

pub const SOLUTION_PACK_SCHEMA_V1: &str = "amos.solution_pack.v1";
pub const TRUST_STORE_SCHEMA_V1: &str = "amos.solution_pack.trust.v1";

const REQUIRED_PARAMETER_IDS: [&str; 7] = [
    "window_start",
    "window_end",
    "comparison_start",
    "comparison_end",
    "population",
    "metric_id",
    "requested_outputs",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignedSolutionPack {
    pub manifest: SolutionPackManifest,
    #[serde(default)]
    pub signatures: Vec<PackSignature>,
}

impl SignedSolutionPack {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|error| {
            AmosError::Storage(format!(
                "failed to read solution pack {}: {error}",
                path.display()
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AmosError::Validation(format!(
                "invalid solution pack JSON {}: {error}",
                path.display()
            ))
        })
    }

    pub fn save_pretty(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let bytes = serde_json::to_vec_pretty(self)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                AmosError::Validation("solution-pack output needs a UTF-8 file name".into())
            })?;
        let temporary = parent.join(format!(
            ".{file_name}.tmp-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| {
                    AmosError::Storage(format!(
                        "failed to create temporary solution pack {}: {error}",
                        temporary.display()
                    ))
                })?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path).map_err(|error| {
                AmosError::Storage(format!(
                    "failed to promote signed solution pack {}: {error}",
                    path.display()
                ))
            })?;
            if let Ok(directory) = fs::File::open(parent) {
                directory.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn manifest_hash(&self) -> Result<String> {
        content_hash(&self.manifest)
    }

    pub fn sign(&mut self, key_id: impl Into<String>, private_key: &[u8; 32]) -> Result<()> {
        self.manifest.validate_contract()?;
        let key_id = key_id.into();
        validate_identifier("signature key_id", &key_id)?;
        let signing_key = SigningKey::from_bytes(private_key);
        let signature = signing_key.sign(&serde_json::to_vec(&self.manifest)?);
        self.signatures.retain(|item| item.key_id != key_id);
        self.signatures.push(PackSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            key_id,
            signature_hex: hex::encode(signature.to_bytes()),
        });
        self.signatures
            .sort_by(|left, right| left.key_id.cmp(&right.key_id));
        Ok(())
    }

    pub fn verify_for_activation(
        &self,
        trust_store: &TrustStore,
        tenant_id: &str,
        core_version: &str,
        now: DateTime<Utc>,
    ) -> Result<VerifiedSolutionPack> {
        self.manifest.validate_contract()?;
        trust_store.validate()?;
        if self.signatures.is_empty() {
            return Err(AmosError::Validation(
                "solution pack is unsigned; activation fails closed".into(),
            ));
        }
        unique_ids(
            "solution-pack signature",
            self.signatures
                .iter()
                .map(|signature| signature.key_id.as_str()),
        )?;
        if !self.manifest.tenant_allowlist.contains(tenant_id) {
            return Err(AmosError::PermissionDenied(format!(
                "solution pack {} is not authorized for tenant {tenant_id}",
                self.manifest.pack_id
            )));
        }
        if self.manifest.status != PackStatus::Approved {
            return Err(AmosError::Validation(format!(
                "solution pack {} is not approved",
                self.manifest.pack_id
            )));
        }
        if self.manifest.owner.approved_at > now {
            return Err(AmosError::Validation(format!(
                "solution pack {} owner approval is in the future",
                self.manifest.pack_id
            )));
        }
        if self.manifest.effective_start > now
            || self.manifest.effective_end.is_some_and(|end| end <= now)
        {
            return Err(AmosError::Validation(format!(
                "solution pack {} is not effective at {now}",
                self.manifest.pack_id
            )));
        }
        let requirement =
            VersionReq::parse(&self.manifest.core_version_requirement).map_err(|error| {
                AmosError::Validation(format!(
                    "invalid core_version_requirement {}: {error}",
                    self.manifest.core_version_requirement
                ))
            })?;
        let version = Version::parse(core_version).map_err(|error| {
            AmosError::Validation(format!("invalid AMOS core version {core_version}: {error}"))
        })?;
        if !requirement.matches(&version) {
            return Err(AmosError::Validation(format!(
                "solution pack {} requires AMOS core {}, but {core_version} is running",
                self.manifest.pack_id, self.manifest.core_version_requirement
            )));
        }

        let message = serde_json::to_vec(&self.manifest)?;
        let mut verified_key = None;
        for signature in &self.signatures {
            if signature.algorithm != SignatureAlgorithm::Ed25519 {
                continue;
            }
            let Some(publisher) = trust_store
                .publishers
                .iter()
                .find(|publisher| publisher.key_id == signature.key_id)
            else {
                continue;
            };
            if !publisher.tenant_allowlist.contains(tenant_id) {
                continue;
            }
            let public_key = decode_exact::<32>(&publisher.public_key_hex, "public key")?;
            let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|error| {
                AmosError::Validation(format!(
                    "invalid public key for trusted publisher {}: {error}",
                    publisher.key_id
                ))
            })?;
            let signature_bytes = decode_exact::<64>(&signature.signature_hex, "signature")?;
            let parsed_signature = Signature::from_bytes(&signature_bytes);
            if verifying_key.verify(&message, &parsed_signature).is_ok() {
                verified_key = Some(publisher.key_id.clone());
                break;
            }
        }
        let verified_key_id = verified_key.ok_or_else(|| {
            AmosError::Validation(
                "solution pack has no valid signature from a tenant-authorized trusted publisher"
                    .into(),
            )
        })?;
        Ok(VerifiedSolutionPack {
            manifest_hash: self.manifest_hash()?,
            pack_id: self.manifest.pack_id.clone(),
            workflow_id: self.manifest.workflow_id.clone(),
            version: self.manifest.version.clone(),
            tenant_id: tenant_id.into(),
            verified_key_id,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SolutionPackManifest {
    pub schema_version: String,
    pub pack_id: String,
    pub workflow_id: String,
    pub version: String,
    pub core_version_requirement: String,
    pub effective_start: DateTime<Utc>,
    pub effective_end: Option<DateTime<Utc>>,
    pub status: PackStatus,
    pub risk_class: RiskClass,
    pub tenant_allowlist: BTreeSet<String>,
    pub owner: PackOwner,
    pub question_families: Vec<QuestionFamily>,
    pub schedule: ScheduleContract,
    pub parameters: BTreeMap<String, ParameterDefinition>,
    pub context_roles: Vec<ContextRoleContract>,
    pub sources: Vec<SourceContract>,
    pub metrics: Vec<MetricContract>,
    pub bank_metadata: Option<BankMetadata>,
    pub plan: PlanContract,
    pub verifier: VerifierContract,
    pub claims: Vec<ClaimContract>,
    pub artifacts: Vec<ArtifactTemplateContract>,
    pub publication: PublicationContract,
    pub retention: RetentionContract,
    pub evaluation_cases: Vec<EvaluationCase>,
}

impl SolutionPackManifest {
    pub fn validate_contract(&self) -> Result<()> {
        if self.schema_version != SOLUTION_PACK_SCHEMA_V1 {
            return Err(AmosError::Validation(format!(
                "unsupported solution-pack schema {}; expected {SOLUTION_PACK_SCHEMA_V1}",
                self.schema_version
            )));
        }
        for (label, value) in [
            ("pack_id", self.pack_id.as_str()),
            ("workflow_id", self.workflow_id.as_str()),
            ("owner.owner_id", self.owner.owner_id.as_str()),
            ("owner.approved_by", self.owner.approved_by.as_str()),
        ] {
            validate_identifier(label, value)?;
        }
        Version::parse(&self.version).map_err(|error| {
            AmosError::Validation(format!(
                "invalid solution pack version {}: {error}",
                self.version
            ))
        })?;
        VersionReq::parse(&self.core_version_requirement).map_err(|error| {
            AmosError::Validation(format!(
                "invalid core_version_requirement {}: {error}",
                self.core_version_requirement
            ))
        })?;
        if self
            .effective_end
            .is_some_and(|end| end <= self.effective_start)
        {
            return Err(AmosError::Validation(
                "effective_end must be later than effective_start".into(),
            ));
        }
        if self.tenant_allowlist.is_empty() {
            return Err(AmosError::Validation(
                "tenant_allowlist must name at least one tenant".into(),
            ));
        }
        if self.owner.role.trim().is_empty() || self.owner.approval_reference.trim().is_empty() {
            return Err(AmosError::Validation(
                "pack owner role and approval reference are required".into(),
            ));
        }
        require_non_empty("question_families", &self.question_families)?;
        require_non_empty("context_roles", &self.context_roles)?;
        require_non_empty("sources", &self.sources)?;
        require_non_empty("metrics", &self.metrics)?;
        require_non_empty("plan.query_shapes", &self.plan.query_shapes)?;
        require_non_empty("claims", &self.claims)?;
        require_non_empty("artifacts", &self.artifacts)?;
        require_non_empty("evaluation_cases", &self.evaluation_cases)?;
        for required in REQUIRED_PARAMETER_IDS {
            if !self.parameters.contains_key(required) {
                return Err(AmosError::Validation(format!(
                    "required workflow parameter {required} is missing"
                )));
            }
        }
        unique_ids(
            "question family",
            self.question_families
                .iter()
                .map(|item| item.family_id.as_str()),
        )?;
        unique_ids(
            "context role",
            self.context_roles.iter().map(|item| item.role_id.as_str()),
        )?;
        unique_ids(
            "source",
            self.sources.iter().map(|item| item.source_id.as_str()),
        )?;
        unique_ids(
            "metric",
            self.metrics.iter().map(|item| item.metric_id.as_str()),
        )?;
        unique_ids(
            "query shape",
            self.plan
                .query_shapes
                .iter()
                .map(|item| item.shape_id.as_str()),
        )?;
        unique_ids(
            "claim",
            self.claims.iter().map(|item| item.claim_type.as_str()),
        )?;
        unique_ids(
            "artifact",
            self.artifacts.iter().map(|item| item.output_id.as_str()),
        )?;
        unique_ids(
            "evaluation case",
            self.evaluation_cases
                .iter()
                .map(|item| item.case_id.as_str()),
        )?;

        let mut normalized_examples = BTreeSet::new();
        for family in &self.question_families {
            validate_identifier("question family ID", &family.family_id)?;
            if family.examples.is_empty() {
                return Err(AmosError::Validation(format!(
                    "question family {} has no examples",
                    family.family_id
                )));
            }
            for parameter in &family.required_parameters {
                if !self.parameters.contains_key(parameter) {
                    return Err(AmosError::Validation(format!(
                        "question family {} references unknown parameter {parameter}",
                        family.family_id
                    )));
                }
            }
            for required in REQUIRED_PARAMETER_IDS {
                if !family.required_parameters.contains(required) {
                    return Err(AmosError::Validation(format!(
                        "question family {} omits required explicit parameter {required}",
                        family.family_id
                    )));
                }
            }
            for example in &family.examples {
                let normalized = normalize_question(example);
                if normalized.is_empty() || !normalized_examples.insert(normalized.clone()) {
                    return Err(AmosError::Validation(format!(
                        "ambiguous or duplicate question example {normalized:?}"
                    )));
                }
            }
        }

        for (parameter_id, parameter) in &self.parameters {
            validate_identifier("parameter ID", parameter_id)?;
            if parameter.description.trim().is_empty() {
                return Err(AmosError::Validation(format!(
                    "parameter {parameter_id} needs a description"
                )));
            }
            if !parameter.allowed_values.is_empty()
                && parameter.parameter_type == ParameterType::Timestamp
            {
                return Err(AmosError::Validation(format!(
                    "timestamp parameter {parameter_id} cannot use enumerated allowed_values"
                )));
            }
        }
        if self.schedule.allowed_time_zones.is_empty()
            || self.schedule.allowed_recurrences.is_empty()
            || !self.schedule.owner_required
        {
            return Err(AmosError::Validation(
                "schedule must require an owner and allow at least one time zone and recurrence"
                    .into(),
            ));
        }

        let role_ids = self
            .context_roles
            .iter()
            .map(|item| item.role_id.as_str())
            .collect::<BTreeSet<_>>();
        if self.plan.allowed_tools.is_empty() || self.plan.max_steps == 0 {
            return Err(AmosError::Validation(
                "plan must allow at least one tool and one step".into(),
            ));
        }
        for role in &self.plan.required_context_roles {
            if !role_ids.contains(role.as_str()) {
                return Err(AmosError::Validation(format!(
                    "plan requires unknown context role {role}"
                )));
            }
        }

        let mut relation_sources = BTreeMap::new();
        for source in &self.sources {
            validate_identifier("source ID", &source.source_id)?;
            validate_identifier("connector type", &source.connector_type)?;
            if !source.read_only {
                return Err(AmosError::Validation(format!(
                    "source {} is not read-only; the banking MVP refuses write-capable packs",
                    source.source_id
                )));
            }
            if source.relations.is_empty() {
                return Err(AmosError::Validation(format!(
                    "source {} has no relations",
                    source.source_id
                )));
            }
            for relation in &source.relations {
                validate_identifier("relation ID", &relation.relation_id)?;
                validate_identifier("physical relation name", &relation.physical_name)?;
                if relation_sources
                    .insert(relation.relation_id.as_str(), source.source_id.as_str())
                    .is_some()
                {
                    return Err(AmosError::Validation(format!(
                        "duplicate relation ID {}",
                        relation.relation_id
                    )));
                }
                if relation.columns.is_empty() || relation.source_version_rule.trim().is_empty() {
                    return Err(AmosError::Validation(format!(
                        "relation {} needs columns and a source-version rule",
                        relation.relation_id
                    )));
                }
                unique_ids(
                    "column",
                    relation.columns.iter().map(|column| column.name.as_str()),
                )?;
            }
        }

        for metric in &self.metrics {
            validate_identifier("metric ID", &metric.metric_id)?;
            if !relation_sources.contains_key(metric.relation_id.as_str()) {
                return Err(AmosError::Validation(format!(
                    "metric {} references unknown relation {}",
                    metric.metric_id, metric.relation_id
                )));
            }
            if !self.parameters.contains_key(&metric.population_parameter) {
                return Err(AmosError::Validation(format!(
                    "metric {} references unknown population parameter {}",
                    metric.metric_id, metric.population_parameter
                )));
            }
            if metric.formula.trim().is_empty()
                || metric.unit.trim().is_empty()
                || metric.owner_approval.trim().is_empty()
            {
                return Err(AmosError::Validation(format!(
                    "metric {} needs formula, unit, and owner approval",
                    metric.metric_id
                )));
            }
            if !metric.tolerance.is_finite()
                || metric.tolerance < 0.0
                || [
                    metric.policy_limit,
                    metric.early_warning,
                    metric.materiality,
                ]
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite())
            {
                return Err(AmosError::Validation(format!(
                    "metric {} has an invalid tolerance or threshold",
                    metric.metric_id
                )));
            }
        }

        if let Some(bank) = &self.bank_metadata {
            for (label, value) in [
                ("institution_type", bank.institution_type.as_str()),
                ("legal_entity", bank.legal_entity.as_str()),
                ("currency", bank.currency.as_str()),
                ("cutoff_policy", bank.cutoff_policy.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(AmosError::Validation(format!(
                        "bank_metadata.{label} must not be empty"
                    )));
                }
            }
        }

        let metric_ids = self
            .metrics
            .iter()
            .map(|metric| metric.metric_id.as_str())
            .collect::<BTreeSet<_>>();
        for shape in &self.plan.query_shapes {
            validate_identifier("query shape ID", &shape.shape_id)?;
            if !self.plan.allowed_tools.contains(&shape.tool) {
                return Err(AmosError::Validation(format!(
                    "query shape {} uses tool {} outside plan.allowed_tools",
                    shape.shape_id, shape.tool
                )));
            }
            let Some(relation_source) = relation_sources.get(shape.relation_id.as_str()) else {
                return Err(AmosError::Validation(format!(
                    "query shape {} references unknown relation {}",
                    shape.shape_id, shape.relation_id
                )));
            };
            if *relation_source != shape.source_id {
                return Err(AmosError::Validation(format!(
                    "query shape {} binds relation {} to the wrong source {}",
                    shape.shape_id, shape.relation_id, shape.source_id
                )));
            }
            if !metric_ids.contains(shape.metric_id.as_str()) {
                return Err(AmosError::Validation(format!(
                    "query shape {} references unknown metric {}",
                    shape.shape_id, shape.metric_id
                )));
            }
            if !starts_with_select(&shape.sql_template) {
                return Err(AmosError::Validation(format!(
                    "query shape {} must be a single read-only SELECT template",
                    shape.shape_id
                )));
            }
            for parameter in placeholders(&shape.sql_template)? {
                if !self.parameters.contains_key(&parameter) {
                    return Err(AmosError::Validation(format!(
                        "query shape {} references unknown template parameter {parameter}",
                        shape.shape_id
                    )));
                }
            }
            if shape.limits.seconds == 0 || shape.limits.rows == 0 || shape.limits.bytes == 0 {
                return Err(AmosError::Validation(format!(
                    "query shape {} has a zero operation limit",
                    shape.shape_id
                )));
            }
        }
        if self.verifier.profile_id.trim().is_empty()
            || self.verifier.reconciliation_rules.is_empty()
            || self.verifier.permitted_repairs.is_empty()
        {
            return Err(AmosError::Validation(
                "verifier profile, reconciliation rules, and bounded repairs are required".into(),
            ));
        }
        let claim_types = self
            .claims
            .iter()
            .map(|claim| claim.claim_type.as_str())
            .collect::<BTreeSet<_>>();
        for claim in &self.claims {
            if claim.value_schema.trim().is_empty() || claim.required_evidence.is_empty() {
                return Err(AmosError::Validation(format!(
                    "claim {} needs a value schema and evidence requirements",
                    claim.claim_type
                )));
            }
        }
        for obligation in &self.verifier.review_obligations {
            if !claim_types.contains(obligation.claim_type.as_str()) {
                return Err(AmosError::Validation(format!(
                    "review obligation references unknown claim type {}",
                    obligation.claim_type
                )));
            }
        }
        if self.publication.destinations.is_empty()
            || self.publication.reviewer_roles.is_empty()
            || !self.publication.review_required
            || !self.publication.acknowledgment_required
        {
            return Err(AmosError::Validation(
                "banking MVP publication requires reviewers, acknowledgment, a destination, and human review"
                    .into(),
            ));
        }
        if !self.retention.legal_hold_supported || self.retention.retention_days == 0 {
            return Err(AmosError::Validation(
                "retention must be non-zero and support legal hold".into(),
            ));
        }
        let output_ids = self
            .artifacts
            .iter()
            .map(|output| output.output_id.as_str())
            .collect::<BTreeSet<_>>();
        for artifact in &self.artifacts {
            if !artifact.evidence_links_required {
                return Err(AmosError::Validation(format!(
                    "artifact {} must require evidence links",
                    artifact.output_id
                )));
            }
            for claim_type in &artifact.required_claim_types {
                if !claim_types.contains(claim_type.as_str()) {
                    return Err(AmosError::Validation(format!(
                        "artifact {} references unknown claim type {claim_type}",
                        artifact.output_id
                    )));
                }
            }
        }
        unique_ids(
            "publication destination",
            self.publication
                .destinations
                .iter()
                .map(|destination| destination.destination_id.as_str()),
        )?;
        for destination in &self.publication.destinations {
            if destination.allowed_artifacts.is_empty() {
                return Err(AmosError::Validation(format!(
                    "publication destination {} has no allowed artifacts",
                    destination.destination_id
                )));
            }
            for output in &destination.allowed_artifacts {
                if !output_ids.contains(output.as_str()) {
                    return Err(AmosError::Validation(format!(
                        "publication destination {} references unknown artifact {output}",
                        destination.destination_id
                    )));
                }
            }
        }
        let metric_values = &self.parameters["metric_id"].allowed_values;
        if metric_values.is_empty()
            || metric_values
                .iter()
                .any(|metric| !metric_ids.contains(metric.as_str()))
        {
            return Err(AmosError::Validation(
                "metric_id allowed_values must contain only declared metrics".into(),
            ));
        }
        let requested_outputs = &self.parameters["requested_outputs"].allowed_values;
        if requested_outputs.is_empty()
            || requested_outputs
                .iter()
                .any(|output| !output_ids.contains(output.as_str()))
        {
            return Err(AmosError::Validation(
                "requested_outputs allowed_values must contain only declared artifacts".into(),
            ));
        }
        for case in &self.evaluation_cases {
            for parameter in REQUIRED_PARAMETER_IDS {
                if !case.parameters.contains_key(parameter) {
                    return Err(AmosError::Validation(format!(
                        "evaluation case {} omits parameter {parameter}",
                        case.case_id
                    )));
                }
            }
            for output in &case.expected_outputs {
                if !output_ids.contains(output.as_str()) {
                    return Err(AmosError::Validation(format!(
                        "evaluation case {} expects unknown output {output}",
                        case.case_id
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackStatus {
    Draft,
    Approved,
    Suspended,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackOwner {
    pub owner_id: String,
    pub role: String,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    pub approval_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuestionFamily {
    pub family_id: String,
    pub examples: Vec<String>,
    pub required_parameters: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleContract {
    pub allowed_time_zones: BTreeSet<String>,
    pub allowed_recurrences: BTreeSet<String>,
    pub owner_required: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    String,
    Timestamp,
    StringList,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParameterDefinition {
    pub parameter_type: ParameterType,
    pub required: bool,
    #[serde(default)]
    pub allowed_values: BTreeSet<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextRoleContract {
    pub role_id: String,
    pub memory_type: MemoryType,
    pub required: bool,
    pub minimum_consistency: ConsistencyClass,
    pub minimum_authority: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceContract {
    pub source_id: String,
    pub connector_type: String,
    pub read_only: bool,
    pub identity_propagation: String,
    pub relations: Vec<RelationContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelationContract {
    pub relation_id: String,
    pub physical_name: String,
    pub schema_version: String,
    pub sensitivity: String,
    pub source_version_rule: String,
    pub columns: Vec<ColumnContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ColumnContract {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub sensitivity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MetricContract {
    pub metric_id: String,
    pub version: String,
    pub relation_id: String,
    pub formula: String,
    pub unit: String,
    pub population_parameter: String,
    pub time_grain: String,
    pub required_filters: Vec<String>,
    pub owner_approval: String,
    pub policy_limit: Option<f64>,
    pub early_warning: Option<f64>,
    pub materiality: Option<f64>,
    pub tolerance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BankMetadata {
    pub institution_type: String,
    pub charter_or_sponsor: String,
    pub regulator_or_jurisdiction: Vec<String>,
    pub legal_entity: String,
    pub business_lines: Vec<String>,
    pub products: Vec<String>,
    pub account_or_gl_hierarchy: String,
    pub currency: String,
    pub cutoff_policy: String,
    pub liquidity_horizons: Vec<String>,
    pub reconciliation_rules: Vec<String>,
    pub scenarios: Vec<String>,
    pub model_versions: Vec<String>,
    pub collateral_or_funding_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanContract {
    pub allowed_tools: BTreeSet<String>,
    pub required_context_roles: BTreeSet<String>,
    pub max_steps: u32,
    pub query_shapes: Vec<QueryShapeContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryShapeContract {
    pub shape_id: String,
    pub purpose: String,
    pub tool: String,
    pub source_id: String,
    pub relation_id: String,
    pub metric_id: String,
    pub sql_template: String,
    pub verifier_profile: String,
    pub permitted_repairs: BTreeSet<String>,
    pub limits: OperationLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VerifierContract {
    pub profile_id: String,
    pub required_filters: Vec<String>,
    pub reconciliation_rules: Vec<String>,
    pub permitted_repairs: BTreeSet<String>,
    pub review_obligations: Vec<ReviewObligation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewObligation {
    pub claim_type: String,
    pub reviewer_role: String,
    pub segregation_of_duties: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaimContract {
    pub claim_type: String,
    pub value_schema: String,
    pub required_evidence: BTreeSet<String>,
    pub review_required: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    DirectAnswer,
    HtmlReport,
    PdfReport,
    Pptx,
    Xlsx,
    Csv,
    Json,
    EvidencePackage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactTemplateContract {
    pub output_id: String,
    pub kind: ArtifactKind,
    pub template_id: String,
    pub version: String,
    pub required_claim_types: BTreeSet<String>,
    pub evidence_links_required: bool,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationContract {
    pub review_required: bool,
    pub reviewer_roles: BTreeSet<String>,
    pub destinations: Vec<PublicationDestination>,
    pub acknowledgment_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicationDestination {
    pub destination_id: String,
    pub destination_type: String,
    pub allowed_artifacts: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetentionContract {
    pub retention_days: u32,
    pub legal_hold_supported: bool,
    pub deletion_mode: String,
    pub export_before_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EvaluationCase {
    pub case_id: String,
    pub question: String,
    pub parameters: BTreeMap<String, serde_json::Value>,
    pub expected_outcome: Outcome,
    pub expected_outputs: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithm {
    Ed25519,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackSignature {
    pub algorithm: SignatureAlgorithm,
    pub key_id: String,
    pub signature_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustStore {
    pub schema_version: String,
    pub publishers: Vec<TrustedPublisher>,
}

impl TrustStore {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|error| {
            AmosError::Storage(format!(
                "failed to read solution-pack trust store {}: {error}",
                path.display()
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AmosError::Validation(format!(
                "invalid solution-pack trust store JSON {}: {error}",
                path.display()
            ))
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != TRUST_STORE_SCHEMA_V1 {
            return Err(AmosError::Validation(format!(
                "unsupported solution-pack trust-store schema {}",
                self.schema_version
            )));
        }
        if self.publishers.is_empty() {
            return Err(AmosError::Validation(
                "solution-pack trust store has no publishers".into(),
            ));
        }
        unique_ids(
            "trusted publisher",
            self.publishers.iter().map(|item| item.key_id.as_str()),
        )?;
        for publisher in &self.publishers {
            validate_identifier("trusted publisher key_id", &publisher.key_id)?;
            let _ = decode_exact::<32>(&publisher.public_key_hex, "public key")?;
            if publisher.tenant_allowlist.is_empty() {
                return Err(AmosError::Validation(format!(
                    "trusted publisher {} has an empty tenant allowlist",
                    publisher.key_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustedPublisher {
    pub key_id: String,
    pub public_key_hex: String,
    pub tenant_allowlist: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedSolutionPack {
    pub manifest_hash: String,
    pub pack_id: String,
    pub workflow_id: String,
    pub version: String,
    pub tenant_id: String,
    pub verified_key_id: String,
}

#[derive(Default)]
pub struct SolutionPackRegistry {
    packs: BTreeMap<(String, String), SignedSolutionPack>,
}

impl SolutionPackRegistry {
    pub fn activate(
        &mut self,
        pack: SignedSolutionPack,
        trust_store: &TrustStore,
        tenant_id: &str,
        core_version: &str,
        now: DateTime<Utc>,
    ) -> Result<VerifiedSolutionPack> {
        let verified = pack.verify_for_activation(trust_store, tenant_id, core_version, now)?;
        for existing in self.packs.values().filter(|existing| {
            existing.manifest.tenant_allowlist.contains(tenant_id)
                && effective_ranges_overlap(&existing.manifest, &pack.manifest)
        }) {
            if existing.manifest.workflow_id == pack.manifest.workflow_id {
                return Err(AmosError::Conflict(format!(
                    "workflow {} has ambiguous overlapping active packs {} and {}",
                    pack.manifest.workflow_id, existing.manifest.pack_id, pack.manifest.pack_id
                )));
            }
            let existing_examples = question_examples(&existing.manifest);
            if let Some(ambiguous) = question_examples(&pack.manifest)
                .intersection(&existing_examples)
                .next()
            {
                return Err(AmosError::Conflict(format!(
                    "question routing example {ambiguous:?} is ambiguous between active packs {} and {}",
                    existing.manifest.pack_id, pack.manifest.pack_id
                )));
            }
        }
        self.packs
            .insert((tenant_id.into(), pack.manifest.pack_id.clone()), pack);
        Ok(verified)
    }
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-/".contains(character))
    {
        return Err(AmosError::Validation(format!(
            "{label} must be a non-empty stable identifier"
        )));
    }
    Ok(())
}

fn require_non_empty<T>(label: &str, values: &[T]) -> Result<()> {
    if values.is_empty() {
        return Err(AmosError::Validation(format!("{label} must not be empty")));
    }
    Ok(())
}

fn unique_ids<'a>(label: &str, values: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(AmosError::Validation(format!(
                "duplicate {label} ID {value}"
            )));
        }
    }
    Ok(())
}

fn decode_exact<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let decoded = hex::decode(value)
        .map_err(|_| AmosError::Validation(format!("{label} must be hexadecimal")))?;
    decoded.try_into().map_err(|bytes: Vec<u8>| {
        AmosError::Validation(format!("{label} must be {N} bytes, found {}", bytes.len()))
    })
}

fn normalize_question(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn starts_with_select(sql: &str) -> bool {
    let normalized = sql.trim_start().to_ascii_lowercase();
    normalized.starts_with("select ")
        && !normalized.contains(';')
        && ![
            " insert ", " update ", " delete ", " drop ", " alter ", " attach ",
        ]
        .iter()
        .any(|token| normalized.contains(token))
}

fn placeholders(template: &str) -> Result<BTreeSet<String>> {
    let mut remaining = template;
    let mut values = BTreeSet::new();
    while let Some(start) = remaining.find("{{") {
        let after_start = &remaining[start + 2..];
        let end = after_start.find("}}").ok_or_else(|| {
            AmosError::Validation("SQL template contains an unclosed parameter".into())
        })?;
        let value = after_start[..end].trim();
        validate_identifier("SQL template parameter", value)?;
        values.insert(value.to_string());
        remaining = &after_start[end + 2..];
    }
    if remaining.contains("}}") {
        return Err(AmosError::Validation(
            "SQL template contains an unmatched closing delimiter".into(),
        ));
    }
    Ok(values)
}

fn effective_ranges_overlap(left: &SolutionPackManifest, right: &SolutionPackManifest) -> bool {
    let left_end = left.effective_end.unwrap_or(DateTime::<Utc>::MAX_UTC);
    let right_end = right.effective_end.unwrap_or(DateTime::<Utc>::MAX_UTC);
    left.effective_start < right_end && right.effective_start < left_end
}

fn question_examples(manifest: &SolutionPackManifest) -> BTreeSet<String> {
    manifest
        .question_families
        .iter()
        .flat_map(|family| family.examples.iter())
        .map(|example| normalize_question(example))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn valid_pack() -> SignedSolutionPack {
        let start = Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap();
        let required_parameters = REQUIRED_PARAMETER_IDS
            .into_iter()
            .map(|value| value.to_string())
            .collect::<BTreeSet<_>>();
        let parameters = REQUIRED_PARAMETER_IDS
            .into_iter()
            .map(|id| {
                (
                    id.into(),
                    ParameterDefinition {
                        parameter_type: if id.ends_with("start") || id.ends_with("end") {
                            ParameterType::Timestamp
                        } else if id == "requested_outputs" {
                            ParameterType::StringList
                        } else {
                            ParameterType::String
                        },
                        required: true,
                        allowed_values: match id {
                            "metric_id" => BTreeSet::from(["weekly_value".into()]),
                            "requested_outputs" => BTreeSet::from(["direct_answer".into()]),
                            "population" => BTreeSet::from(["all".into()]),
                            _ => BTreeSet::new(),
                        },
                        description: format!("Explicit {id}"),
                    },
                )
            })
            .collect();
        SignedSolutionPack {
            manifest: SolutionPackManifest {
                schema_version: SOLUTION_PACK_SCHEMA_V1.into(),
                pack_id: "fixture.weekly_review.v1".into(),
                workflow_id: "weekly_review".into(),
                version: "1.0.0".into(),
                core_version_requirement: ">=0.2.0, <0.3.0".into(),
                effective_start: start,
                effective_end: None,
                status: PackStatus::Approved,
                risk_class: RiskClass::MaterialInternal,
                tenant_allowlist: BTreeSet::from(["tenant_fixture".into()]),
                owner: PackOwner {
                    owner_id: "fixture_owner".into(),
                    role: "data_owner".into(),
                    approved_by: "fixture_owner".into(),
                    approved_at: start,
                    approval_reference: "synthetic-fixture-approval".into(),
                },
                question_families: vec![QuestionFamily {
                    family_id: "weekly_status".into(),
                    examples: vec!["What changed this week?".into()],
                    required_parameters,
                }],
                schedule: ScheduleContract {
                    allowed_time_zones: BTreeSet::from(["UTC".into()]),
                    allowed_recurrences: BTreeSet::from(["weekly".into()]),
                    owner_required: true,
                },
                parameters,
                context_roles: vec![ContextRoleContract {
                    role_id: "metric_definition".into(),
                    memory_type: MemoryType::SemanticDefinition,
                    required: true,
                    minimum_consistency: ConsistencyClass::C1,
                    minimum_authority: "owner_approved".into(),
                }],
                sources: vec![SourceContract {
                    source_id: "warehouse".into(),
                    connector_type: "sqlite_fixture".into(),
                    read_only: true,
                    identity_propagation: "tenant_and_subject".into(),
                    relations: vec![RelationContract {
                        relation_id: "weekly_values".into(),
                        physical_name: "weekly_values".into(),
                        schema_version: "v1".into(),
                        sensitivity: "internal".into(),
                        source_version_rule: "schema_and_snapshot_hash".into(),
                        columns: vec![ColumnContract {
                            name: "value".into(),
                            data_type: "real".into(),
                            nullable: false,
                            sensitivity: "internal".into(),
                        }],
                    }],
                }],
                metrics: vec![MetricContract {
                    metric_id: "weekly_value".into(),
                    version: "1".into(),
                    relation_id: "weekly_values".into(),
                    formula: "sum(value)".into(),
                    unit: "count".into(),
                    population_parameter: "population".into(),
                    time_grain: "week".into(),
                    required_filters: vec!["recorded_at in requested window".into()],
                    owner_approval: "synthetic-fixture-approval".into(),
                    policy_limit: None,
                    early_warning: None,
                    materiality: Some(1.0),
                    tolerance: 0.0,
                }],
                bank_metadata: None,
                plan: PlanContract {
                    allowed_tools: BTreeSet::from(["sql.readonly.v1".into()]),
                    required_context_roles: BTreeSet::from(["metric_definition".into()]),
                    max_steps: 1,
                    query_shapes: vec![QueryShapeContract {
                        shape_id: "weekly_summary".into(),
                        purpose: "Compute an approved weekly value".into(),
                        tool: "sql.readonly.v1".into(),
                        source_id: "warehouse".into(),
                        relation_id: "weekly_values".into(),
                        metric_id: "weekly_value".into(),
                        sql_template:
                            "SELECT SUM(value) AS value FROM weekly_values WHERE recorded_at >= '{{window_start}}' AND recorded_at < '{{window_end}}'"
                                .into(),
                        verifier_profile: "weekly_review.v1".into(),
                        permitted_repairs: BTreeSet::from(["rename_declared_column".into()]),
                        limits: OperationLimits {
                            seconds: 10,
                            rows: 100,
                            bytes: 100_000,
                        },
                    }],
                },
                verifier: VerifierContract {
                    profile_id: "weekly_review.v1".into(),
                    required_filters: vec!["window_start".into(), "window_end".into()],
                    reconciliation_rules: vec!["no_duplicate_periods".into()],
                    permitted_repairs: BTreeSet::from(["rename_declared_column".into()]),
                    review_obligations: vec![ReviewObligation {
                        claim_type: "metric_value".into(),
                        reviewer_role: "reviewer".into(),
                        segregation_of_duties: true,
                    }],
                },
                claims: vec![ClaimContract {
                    claim_type: "metric_value".into(),
                    value_schema: "decimal_with_unit".into(),
                    required_evidence: BTreeSet::from(["execution".into(), "metric".into()]),
                    review_required: true,
                }],
                artifacts: vec![ArtifactTemplateContract {
                    output_id: "direct_answer".into(),
                    kind: ArtifactKind::DirectAnswer,
                    template_id: "weekly_answer.v1".into(),
                    version: "1".into(),
                    required_claim_types: BTreeSet::from(["metric_value".into()]),
                    evidence_links_required: true,
                    editable: false,
                }],
                publication: PublicationContract {
                    review_required: true,
                    reviewer_roles: BTreeSet::from(["reviewer".into()]),
                    destinations: vec![PublicationDestination {
                        destination_id: "local_review".into(),
                        destination_type: "local_object_store".into(),
                        allowed_artifacts: BTreeSet::from(["direct_answer".into()]),
                    }],
                    acknowledgment_required: true,
                },
                retention: RetentionContract {
                    retention_days: 30,
                    legal_hold_supported: true,
                    deletion_mode: "tombstone_then_purge".into(),
                    export_before_delete: true,
                },
                evaluation_cases: vec![EvaluationCase {
                    case_id: "weekly_review_pass".into(),
                    question: "What changed this week?".into(),
                    parameters: BTreeMap::from([
                        ("window_start".into(), json!("2026-08-13T00:00:00Z")),
                        ("window_end".into(), json!("2026-08-20T00:00:00Z")),
                        ("comparison_start".into(), json!("2026-08-06T00:00:00Z")),
                        ("comparison_end".into(), json!("2026-08-13T00:00:00Z")),
                        ("population".into(), json!("all")),
                        ("metric_id".into(), json!("weekly_value")),
                        ("requested_outputs".into(), json!(["direct_answer"])),
                    ]),
                    expected_outcome: Outcome::NeedsReview,
                    expected_outputs: BTreeSet::from(["direct_answer".into()]),
                }],
            },
            signatures: vec![],
        }
    }

    fn signed_pack() -> (SignedSolutionPack, TrustStore) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut pack = valid_pack();
        pack.sign("fixture-publisher", &key.to_bytes()).unwrap();
        let trust = TrustStore {
            schema_version: TRUST_STORE_SCHEMA_V1.into(),
            publishers: vec![TrustedPublisher {
                key_id: "fixture-publisher".into(),
                public_key_hex: hex::encode(key.verifying_key().to_bytes()),
                tenant_allowlist: BTreeSet::from(["tenant_fixture".into()]),
            }],
        };
        (pack, trust)
    }

    #[test]
    fn signed_approved_pack_verifies_for_authorized_tenant() {
        let (pack, trust) = signed_pack();
        let verified = pack
            .verify_for_activation(
                &trust,
                "tenant_fixture",
                "0.2.0",
                Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap(),
            )
            .unwrap();
        assert_eq!(verified.workflow_id, "weekly_review");
        assert_eq!(verified.verified_key_id, "fixture-publisher");
    }

    #[test]
    fn unsigned_tampered_incompatible_and_unauthorized_packs_fail_closed() {
        let (pack, trust) = signed_pack();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();

        let mut unsigned = pack.clone();
        unsigned.signatures.clear();
        assert!(
            unsigned
                .verify_for_activation(&trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("unsigned")
        );

        let mut tampered = pack.clone();
        tampered.manifest.metrics[0].formula = "sum(value) + 1".into();
        assert!(
            tampered
                .verify_for_activation(&trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("no valid signature")
        );

        assert!(
            pack.verify_for_activation(&trust, "tenant_fixture", "1.0.0", now)
                .unwrap_err()
                .to_string()
                .contains("requires AMOS core")
        );
        assert!(matches!(
            pack.verify_for_activation(&trust, "other_tenant", "0.2.0", now),
            Err(AmosError::PermissionDenied(_))
        ));

        let mut duplicate_signature = pack.clone();
        duplicate_signature
            .signatures
            .push(duplicate_signature.signatures[0].clone());
        assert!(
            duplicate_signature
                .verify_for_activation(&trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("duplicate solution-pack signature")
        );

        let mut future_approval = valid_pack();
        future_approval.manifest.owner.approved_at =
            Utc.with_ymd_and_hms(2026, 8, 22, 0, 0, 0).unwrap();
        future_approval.sign("fixture-publisher", &[7; 32]).unwrap();
        assert!(
            future_approval
                .verify_for_activation(&trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("approval is in the future")
        );
    }

    #[test]
    fn ambiguous_or_write_capable_contracts_are_rejected() {
        let mut pack = valid_pack();
        pack.manifest.question_families.push(QuestionFamily {
            family_id: "duplicate".into(),
            examples: vec!["  WHAT changed   this week? ".into()],
            required_parameters: REQUIRED_PARAMETER_IDS
                .into_iter()
                .map(str::to_string)
                .collect(),
        });
        assert!(
            pack.manifest
                .validate_contract()
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );

        let mut write_capable = valid_pack();
        write_capable.manifest.sources[0].read_only = false;
        assert!(
            write_capable
                .manifest
                .validate_contract()
                .unwrap_err()
                .to_string()
                .contains("not read-only")
        );
    }

    #[test]
    fn registry_rejects_overlapping_workflow_activation() {
        let (pack, trust) = signed_pack();
        let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();
        let mut registry = SolutionPackRegistry::default();
        registry
            .activate(pack.clone(), &trust, "tenant_fixture", "0.2.0", now)
            .unwrap();
        assert!(
            registry
                .activate(pack, &trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("ambiguous overlapping")
        );

        let (first, trust) = signed_pack();
        let mut other_workflow = valid_pack();
        other_workflow.manifest.pack_id = "fixture.other_review.v1".into();
        other_workflow.manifest.workflow_id = "other_weekly_review".into();
        other_workflow.sign("fixture-publisher", &[7; 32]).unwrap();
        let mut registry = SolutionPackRegistry::default();
        registry
            .activate(first, &trust, "tenant_fixture", "0.2.0", now)
            .unwrap();
        assert!(
            registry
                .activate(other_workflow, &trust, "tenant_fixture", "0.2.0", now)
                .unwrap_err()
                .to_string()
                .contains("question routing example")
        );
    }

    #[test]
    fn strict_json_rejects_unknown_manifest_fields() {
        let pack = valid_pack();
        let mut value = serde_json::to_value(pack).unwrap();
        value["manifest"]["unreviewed_behavior"] = json!(true);
        let error = serde_json::from_value::<SignedSolutionPack>(value).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}
