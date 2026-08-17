use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AmosError, Result, domain::PlanStep};

pub const TOOL_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolAvailability {
    PlanStep,
    EmbeddedRuntime,
    ContractOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolRuntimeKind {
    Sql,
    NativeRust,
    Spark,
    R,
    Python,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    DenyAll,
    SourceOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolRuntimeSpec {
    pub kind: ToolRuntimeKind,
    pub audience: String,
    pub entrypoint: Option<String>,
    pub image_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolSourcePolicy {
    pub required: bool,
    pub read_only: bool,
    pub relations_parameter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolResourceLimits {
    pub max_seconds: u64,
    pub max_rows: u64,
    pub max_bytes: u64,
    pub max_memory_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeterminismPolicy {
    pub deterministic: bool,
    pub seed_parameter: Option<String>,
    pub records_runtime_version: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolManifest {
    pub schema_version: u32,
    pub tool_id: String,
    pub display_name: String,
    pub description: String,
    pub availability: ToolAvailability,
    pub evaluation_only: bool,
    pub runtime: ToolRuntimeSpec,
    pub operations: BTreeSet<String>,
    pub source_policy: ToolSourcePolicy,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub resource_limits: ToolResourceLimits,
    pub network_policy: NetworkPolicy,
    pub determinism: DeterminismPolicy,
    pub verifier_profile: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ToolCatalogEntry {
    pub tool_id: String,
    pub display_name: String,
    pub description: String,
    pub availability: ToolAvailability,
    pub evaluation_only: bool,
    pub executable_plan_step: bool,
    pub runtime_kind: ToolRuntimeKind,
    pub operations: BTreeSet<String>,
    pub source_policy: ToolSourcePolicy,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub resource_limits: ToolResourceLimits,
    pub network_policy: NetworkPolicy,
    pub determinism: DeterminismPolicy,
    pub verifier_profile: String,
}

impl ToolManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let body = fs::read(path.as_ref()).map_err(|error| {
            AmosError::Storage(format!(
                "failed to read tool manifest {}: {error}",
                path.as_ref().display()
            ))
        })?;
        let manifest: Self = serde_json::from_slice(&body)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_json(body: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(body)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != TOOL_MANIFEST_SCHEMA_VERSION {
            return Err(AmosError::Validation(format!(
                "unsupported tool manifest schema version {}; expected {}",
                self.schema_version, TOOL_MANIFEST_SCHEMA_VERSION
            )));
        }
        validate_tool_id(&self.tool_id)?;
        validate_non_empty("display_name", &self.display_name)?;
        validate_non_empty("description", &self.description)?;
        validate_identifier("runtime audience", &self.runtime.audience)?;
        validate_non_empty("verifier_profile", &self.verifier_profile)?;
        if self.operations.is_empty()
            || self
                .operations
                .iter()
                .any(|operation| validate_identifier("operation", operation).is_err())
        {
            return Err(AmosError::Validation(
                "tool operations must contain one or more lowercase identifiers".into(),
            ));
        }
        if self.source_policy.read_only && self.operations.contains("write") {
            return Err(AmosError::Validation(
                "a read-only tool cannot declare the write operation".into(),
            ));
        }
        if self.source_policy.required
            && self
                .source_policy
                .relations_parameter
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(AmosError::Validation(
                "a source-backed tool must declare its relations_parameter".into(),
            ));
        }
        if self.resource_limits.max_seconds == 0
            || self.resource_limits.max_rows == 0
            || self.resource_limits.max_bytes == 0
            || self.resource_limits.max_memory_mb == 0
        {
            return Err(AmosError::Validation(
                "tool resource limits must all be greater than zero".into(),
            ));
        }
        if let Some(entrypoint) = &self.runtime.entrypoint {
            validate_non_empty("runtime entrypoint", entrypoint)?;
        }
        if let Some(digest) = &self.runtime.image_digest {
            validate_image_digest(digest)?;
        }
        if self.availability == ToolAvailability::PlanStep
            && !matches!(
                self.runtime.kind,
                ToolRuntimeKind::Sql | ToolRuntimeKind::NativeRust
            )
            && self.runtime.image_digest.is_none()
            && !self.evaluation_only
        {
            return Err(AmosError::Validation(
                "an executable external tool must pin an OCI image digest".into(),
            ));
        }
        if let Some(seed_parameter) = &self.determinism.seed_parameter {
            validate_identifier("seed_parameter", seed_parameter)?;
        }
        validate_schema(&self.parameter_schema, "parameter_schema")?;
        validate_closed_objects(&self.parameter_schema, "parameter_schema")?;
        if self.source_policy.required {
            validate_relations_contract(
                &self.parameter_schema,
                self.source_policy
                    .relations_parameter
                    .as_deref()
                    .expect("source-backed contract checked above"),
            )?;
        }
        validate_schema(&self.output_schema, "output_schema")?;
        Ok(())
    }

    pub fn validate_parameters(&self, parameters: &Value) -> Result<()> {
        validate_instance(parameters, &self.parameter_schema, "parameters")
    }

    pub fn validate_output(&self, output: &Value) -> Result<()> {
        validate_instance(output, &self.output_schema, "output")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolImplementation {
    SqlReadOnly,
    ExternalToolbox,
}

#[derive(Debug, Clone)]
struct RegisteredTool {
    manifest: ToolManifest,
    implementation: Option<ToolImplementation>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn builtins() -> Result<Self> {
        let mut registry = Self::default();
        registry.register_builtin(
            ToolManifest::from_json(include_str!("../tool-packs/builtin/sql.readonly.v1.json"))?,
            ToolImplementation::SqlReadOnly,
        )?;
        registry.register_contract(ToolManifest::from_json(include_str!(
            "../tool-packs/builtin/stats.rate_comparison.v1.json"
        ))?)?;
        registry.register_contract(ToolManifest::from_json(include_str!(
            "../tool-packs/builtin/chart.timeseries.v1.json"
        ))?)?;
        for manifest in [
            include_str!("../tool-packs/executable/spark.dataframe.aggregate.v1.json"),
            include_str!("../tool-packs/executable/r.statistics.v1.json"),
            include_str!("../tool-packs/executable/python.dataframe.aggregate.v1.json"),
            include_str!("../tool-packs/executable/polars.dataframe.aggregate.v1.json"),
            include_str!("../tool-packs/executable/duckdb.readonly.v1.json"),
            include_str!("../tool-packs/executable/dbt.manifest.validate.v1.json"),
            include_str!("../tool-packs/executable/stats.regression.v1.json"),
            include_str!("../tool-packs/executable/stats.forecast.v1.json"),
            include_str!("../tool-packs/executable/stats.pca.v1.json"),
            include_str!("../tool-packs/executable/spreadsheet.xlsx.v1.json"),
            include_str!("../tool-packs/executable/presentation.pptx.v1.json"),
            include_str!("../tool-packs/executable/notebook.inspect.v1.json"),
        ] {
            registry.register_builtin(
                ToolManifest::from_json(manifest)?,
                ToolImplementation::ExternalToolbox,
            )?;
        }
        Ok(registry)
    }

    pub fn register_contract(&mut self, manifest: ToolManifest) -> Result<()> {
        self.register(manifest, None)
    }

    pub fn register_builtin(
        &mut self,
        manifest: ToolManifest,
        implementation: ToolImplementation,
    ) -> Result<()> {
        if manifest.availability != ToolAvailability::PlanStep {
            return Err(AmosError::Validation(
                "a built-in plan-step implementation requires plan_step availability".into(),
            ));
        }
        self.register(manifest, Some(implementation))
    }

    fn register(
        &mut self,
        manifest: ToolManifest,
        implementation: Option<ToolImplementation>,
    ) -> Result<()> {
        manifest.validate()?;
        let tool_id = manifest.tool_id.clone();
        if self.tools.contains_key(&tool_id) {
            return Err(AmosError::Conflict(format!(
                "tool {tool_id} is already registered"
            )));
        }
        self.tools.insert(
            tool_id,
            RegisteredTool {
                manifest,
                implementation,
            },
        );
        Ok(())
    }

    pub fn get(&self, tool_id: &str) -> Result<&ToolManifest> {
        self.tools
            .get(tool_id)
            .map(|registered| &registered.manifest)
            .ok_or_else(|| AmosError::Validation(format!("tool {tool_id} is not registered")))
    }

    pub fn list(&self) -> Vec<&ToolManifest> {
        self.tools
            .values()
            .map(|registered| &registered.manifest)
            .collect()
    }

    pub fn catalog(&self) -> Vec<ToolCatalogEntry> {
        self.tools
            .values()
            .map(|registered| ToolCatalogEntry {
                tool_id: registered.manifest.tool_id.clone(),
                display_name: registered.manifest.display_name.clone(),
                description: registered.manifest.description.clone(),
                availability: registered.manifest.availability,
                evaluation_only: registered.manifest.evaluation_only,
                executable_plan_step: registered.implementation.is_some(),
                runtime_kind: registered.manifest.runtime.kind,
                operations: registered.manifest.operations.clone(),
                source_policy: registered.manifest.source_policy.clone(),
                parameter_schema: registered.manifest.parameter_schema.clone(),
                output_schema: registered.manifest.output_schema.clone(),
                resource_limits: registered.manifest.resource_limits.clone(),
                network_policy: registered.manifest.network_policy,
                determinism: registered.manifest.determinism.clone(),
                verifier_profile: registered.manifest.verifier_profile.clone(),
            })
            .collect()
    }

    pub fn implementation(&self, tool_id: &str) -> Result<ToolImplementation> {
        let registered = self
            .tools
            .get(tool_id)
            .ok_or_else(|| AmosError::Validation(format!("tool {tool_id} is not registered")))?;
        registered.implementation.ok_or_else(|| {
            AmosError::Validation(format!(
                "tool {tool_id} has a validated contract but no executable plan-step implementation"
            ))
        })
    }

    pub fn is_executable(&self, tool_id: &str) -> Result<bool> {
        Ok(self
            .tools
            .get(tool_id)
            .ok_or_else(|| AmosError::Validation(format!("tool {tool_id} is not registered")))?
            .implementation
            .is_some())
    }

    pub fn validate_step(&self, step: &PlanStep) -> Result<()> {
        let manifest = self.get(&step.tool)?;
        if manifest.availability != ToolAvailability::PlanStep {
            return Err(AmosError::Validation(format!(
                "tool {} is {:?}, not an executable plan-step tool",
                step.tool, manifest.availability
            )));
        }
        manifest.validate_parameters(&step.parameters)?;
        if manifest.source_policy.required && step.source_id.trim().is_empty() {
            return Err(AmosError::Validation(format!(
                "tool {} requires a source_id",
                step.tool
            )));
        }
        if step.limits.seconds == 0
            || step.limits.rows == 0
            || step.limits.bytes == 0
            || step.limits.seconds > manifest.resource_limits.max_seconds
            || step.limits.rows > manifest.resource_limits.max_rows
            || step.limits.bytes > manifest.resource_limits.max_bytes
        {
            return Err(AmosError::Validation(format!(
                "step {} exceeds the registered limits for {}",
                step.step_id, step.tool
            )));
        }
        Ok(())
    }

    pub fn validate_allowed_tools<'a>(
        &self,
        tool_ids: impl IntoIterator<Item = &'a String>,
    ) -> Result<()> {
        for tool_id in tool_ids {
            let manifest = self.get(tool_id)?;
            if manifest.availability == ToolAvailability::ContractOnly {
                return Err(AmosError::Validation(format!(
                    "tool {tool_id} is contract_only and cannot be activated by a task definition"
                )));
            }
        }
        Ok(())
    }
}

fn validate_relations_contract(schema: &Value, parameter: &str) -> Result<()> {
    if parameter != "relations" {
        return Err(AmosError::Validation(
            "source-backed tools currently require the canonical relations parameter".into(),
        ));
    }
    let object = schema.as_object().expect("schema validated");
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .expect("object parameter schema validated");
    let relation_schema = properties.get(parameter).and_then(Value::as_object);
    let valid = relation_schema.is_some_and(|relation_schema| {
        relation_schema.get("type").and_then(Value::as_str) == Some("array")
            && relation_schema
                .get("items")
                .and_then(Value::as_object)
                .and_then(|items| items.get("type"))
                .and_then(Value::as_str)
                == Some("string")
            && relation_schema
                .get("minItems")
                .and_then(Value::as_u64)
                .is_some_and(|minimum| minimum >= 1)
    });
    let required = object
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| {
            required
                .iter()
                .any(|value| value.as_str() == Some(parameter))
        });
    if !valid || !required {
        return Err(AmosError::Validation(
            "source-backed parameter schemas must require a non-empty string array named relations"
                .into(),
        ));
    }
    Ok(())
}

fn validate_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(AmosError::Validation(format!(
            "{name} must be non-empty and must not have surrounding whitespace"
        )));
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    validate_non_empty(name, value)?;
    if value.bytes().any(|byte| {
        !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'.' | b'-' | b'_' | b':'))
    }) {
        return Err(AmosError::Validation(format!(
            "{name} must use lowercase ASCII letters, numbers, dots, colons, underscores, or hyphens"
        )));
    }
    Ok(())
}

fn validate_tool_id(tool_id: &str) -> Result<()> {
    validate_identifier("tool_id", tool_id)?;
    let Some(version) = tool_id.rsplit('.').next() else {
        return Err(AmosError::Validation("tool_id has no version".into()));
    };
    if tool_id.matches('.').count() < 2
        || !version.strip_prefix('v').is_some_and(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(AmosError::Validation(
            "tool_id must be a namespaced lowercase identifier ending in .v<number>".into(),
        ));
    }
    Ok(())
}

fn validate_image_digest(digest: &str) -> Result<()> {
    let Some(encoded) = digest.strip_prefix("sha256:") else {
        return Err(AmosError::Validation(
            "image_digest must use a sha256: digest".into(),
        ));
    };
    if encoded.len() != 64
        || encoded
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(AmosError::Validation(
            "image_digest must contain 64 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn validate_schema(schema: &Value, name: &str) -> Result<()> {
    let object = schema
        .as_object()
        .ok_or_else(|| AmosError::Validation(format!("{name} must be a JSON Schema object")))?;
    let schema_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| AmosError::Validation(format!("{name} must declare a supported type")))?;
    if !matches!(
        schema_type,
        "object" | "array" | "string" | "integer" | "number" | "boolean" | "scalar"
    ) {
        return Err(AmosError::Validation(format!(
            "{name} declares unsupported type {schema_type}"
        )));
    }
    let mut allowed_keywords = BTreeSet::from(["type", "enum"]);
    match schema_type {
        "object" => allowed_keywords.extend(["properties", "required", "additionalProperties"]),
        "array" => allowed_keywords.extend(["items", "minItems", "maxItems", "uniqueItems"]),
        "string" => allowed_keywords.extend(["minLength", "maxLength"]),
        "integer" | "number" => allowed_keywords.extend(["minimum", "maximum"]),
        "boolean" => {}
        "scalar" => {}
        _ => unreachable!("supported schema type checked above"),
    }
    if let Some(keyword) = object
        .keys()
        .find(|keyword| !allowed_keywords.contains(keyword.as_str()))
    {
        return Err(AmosError::Validation(format!(
            "{name} uses unsupported schema keyword {keyword}"
        )));
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| {
                AmosError::Validation(format!("{name}.enum must be a non-empty array"))
            })?;
        if values
            .iter()
            .any(|value| !value_matches_schema_type(value, schema_type))
        {
            return Err(AmosError::Validation(format!(
                "{name}.enum contains a value that does not match type {schema_type}"
            )));
        }
    }
    if schema_type == "object" {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                AmosError::Validation(format!("{name} object schema requires properties"))
            })?;
        for (property, property_schema) in properties {
            validate_identifier("schema property", property)?;
            validate_schema(property_schema, &format!("{name}.properties.{property}"))?;
        }
        if let Some(required) = object.get("required") {
            let required = required.as_array().ok_or_else(|| {
                AmosError::Validation(format!("{name}.required must be an array"))
            })?;
            let mut seen = BTreeSet::new();
            for property in required {
                let property = property.as_str().ok_or_else(|| {
                    AmosError::Validation(format!("{name}.required must contain strings"))
                })?;
                if !properties.contains_key(property) {
                    return Err(AmosError::Validation(format!(
                        "{name}.required references unknown property {property}"
                    )));
                }
                if !seen.insert(property) {
                    return Err(AmosError::Validation(format!(
                        "{name}.required repeats property {property}"
                    )));
                }
            }
        }
        if !object
            .get("additionalProperties")
            .is_some_and(Value::is_boolean)
        {
            return Err(AmosError::Validation(format!(
                "{name} object schemas must declare boolean additionalProperties"
            )));
        }
    }
    if schema_type == "array" {
        let items = object
            .get("items")
            .ok_or_else(|| AmosError::Validation(format!("{name} array schema requires items")))?;
        validate_schema(items, &format!("{name}.items"))?;
        validate_schema_range(object, "minItems", "maxItems", name)?;
        if object
            .get("uniqueItems")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(AmosError::Validation(format!(
                "{name}.uniqueItems must be a boolean"
            )));
        }
    }
    if schema_type == "string" {
        validate_schema_range(object, "minLength", "maxLength", name)?;
    }
    if matches!(schema_type, "integer" | "number") {
        let minimum = schema_number(object.get("minimum"), &format!("{name}.minimum"))?;
        let maximum = schema_number(object.get("maximum"), &format!("{name}.maximum"))?;
        if minimum
            .zip(maximum)
            .is_some_and(|(minimum, maximum)| minimum > maximum)
        {
            return Err(AmosError::Validation(format!(
                "{name} minimum cannot exceed maximum"
            )));
        }
    }
    Ok(())
}

fn value_matches_schema_type(value: &Value, schema_type: &str) -> bool {
    match schema_type {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "scalar" => value.is_null() || value.is_string() || value.is_number() || value.is_boolean(),
        _ => false,
    }
}

fn validate_schema_range(
    schema: &serde_json::Map<String, Value>,
    minimum_name: &str,
    maximum_name: &str,
    path: &str,
) -> Result<()> {
    let minimum = schema
        .get(minimum_name)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                AmosError::Validation(format!("{path}.{minimum_name} must be an integer"))
            })
        })
        .transpose()?;
    let maximum = schema
        .get(maximum_name)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                AmosError::Validation(format!("{path}.{maximum_name} must be an integer"))
            })
        })
        .transpose()?;
    if minimum
        .zip(maximum)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return Err(AmosError::Validation(format!(
            "{path} minimum cannot exceed maximum"
        )));
    }
    Ok(())
}

fn schema_number(value: Option<&Value>, path: &str) -> Result<Option<f64>> {
    value
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| AmosError::Validation(format!("{path} must be a finite number")))
        })
        .transpose()
}

fn validate_closed_objects(schema: &Value, name: &str) -> Result<()> {
    let object = schema
        .as_object()
        .ok_or_else(|| AmosError::Validation(format!("{name} must be a JSON Schema object")))?;
    match object.get("type").and_then(Value::as_str) {
        Some("object") => {
            if object.get("additionalProperties").and_then(Value::as_bool) != Some(false) {
                return Err(AmosError::Validation(format!(
                    "{name} parameter objects must set additionalProperties to false"
                )));
            }
            for (property, property_schema) in object
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                validate_closed_objects(property_schema, &format!("{name}.properties.{property}"))?;
            }
        }
        Some("array") => {
            if let Some(items) = object.get("items") {
                validate_closed_objects(items, &format!("{name}.items"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_instance(instance: &Value, schema: &Value, path: &str) -> Result<()> {
    let schema_object = schema.as_object().ok_or_else(|| {
        AmosError::Validation(format!("invalid registered schema while validating {path}"))
    })?;
    let schema_type = schema_object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| AmosError::Validation(format!("registered schema has no type at {path}")))?;
    let valid_type = value_matches_schema_type(instance, schema_type);
    if !valid_type {
        return Err(AmosError::Validation(format!(
            "{path} must have type {schema_type}"
        )));
    }
    if let Some(values) = schema_object.get("enum").and_then(Value::as_array)
        && !values.contains(instance)
    {
        return Err(AmosError::Validation(format!(
            "{path} is not one of the allowed values"
        )));
    }
    match schema_type {
        "object" => {
            let instance = instance.as_object().expect("type checked");
            let properties = schema_object
                .get("properties")
                .and_then(Value::as_object)
                .expect("manifest validation requires properties");
            if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
                for property in required.iter().filter_map(Value::as_str) {
                    if !instance.contains_key(property) {
                        return Err(AmosError::Validation(format!(
                            "{path} is missing required property {property}"
                        )));
                    }
                }
            }
            for (property, value) in instance {
                let Some(property_schema) = properties.get(property) else {
                    if schema_object
                        .get("additionalProperties")
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        continue;
                    }
                    return Err(AmosError::Validation(format!(
                        "{path} contains undeclared property {property}"
                    )));
                };
                validate_instance(value, property_schema, &format!("{path}.{property}"))?;
            }
        }
        "array" => {
            let values = instance.as_array().expect("type checked");
            validate_length(values.len(), schema_object, path)?;
            if schema_object.get("uniqueItems").and_then(Value::as_bool) == Some(true) {
                for (index, value) in values.iter().enumerate() {
                    if values[..index].contains(value) {
                        return Err(AmosError::Validation(format!(
                            "{path} must not contain duplicate items"
                        )));
                    }
                }
            }
            let item_schema = schema_object.get("items").expect("schema validated");
            for (index, value) in values.iter().enumerate() {
                validate_instance(value, item_schema, &format!("{path}[{index}]"))?;
            }
        }
        "string" => {
            validate_length(
                instance.as_str().expect("type checked").chars().count(),
                schema_object,
                path,
            )?;
        }
        "integer" | "number" => {
            let number = instance
                .as_f64()
                .ok_or_else(|| AmosError::Validation(format!("{path} is not finite")))?;
            if schema_object
                .get("minimum")
                .and_then(Value::as_f64)
                .is_some_and(|minimum| number < minimum)
                || schema_object
                    .get("maximum")
                    .and_then(Value::as_f64)
                    .is_some_and(|maximum| number > maximum)
            {
                return Err(AmosError::Validation(format!(
                    "{path} is outside the declared numeric range"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_length(
    length: usize,
    schema: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<()> {
    let minimum = schema
        .get("minLength")
        .or_else(|| schema.get("minItems"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let maximum = schema
        .get("maxLength")
        .or_else(|| schema.get("maxItems"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX);
    let length = length as u64;
    if length < minimum || length > maximum {
        return Err(AmosError::Validation(format!(
            "{path} length is outside the declared range"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::OperationLimits;

    fn sql_step() -> PlanStep {
        PlanStep {
            step_id: "summary".into(),
            purpose: "bounded summary".into(),
            tool: "sql.readonly.v1".into(),
            source_id: "warehouse".into(),
            input_object_ids: vec![],
            parameter_schema: "summary.v1".into(),
            parameters: json!({
                "sql": "SELECT COUNT(*) FROM events",
                "relations": ["analytics"]
            }),
            expected_output_schema: "rows.v1".into(),
            limits: OperationLimits {
                seconds: 5,
                rows: 100,
                bytes: 10_000,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: "sql.readonly.v1".into(),
        }
    }

    #[test]
    fn builtins_are_strict_and_distinguish_execution_availability() {
        let registry = ToolRegistry::builtins().unwrap();
        assert_eq!(registry.list().len(), 15);
        assert_eq!(
            registry.implementation("sql.readonly.v1").unwrap(),
            ToolImplementation::SqlReadOnly
        );
        assert!(registry.implementation("stats.rate_comparison.v1").is_err());
        assert_eq!(
            registry
                .implementation("spark.dataframe.aggregate.v1")
                .unwrap(),
            ToolImplementation::ExternalToolbox
        );
        registry.validate_step(&sql_step()).unwrap();
    }

    #[test]
    fn step_validation_rejects_unknown_parameters_and_excess_limits() {
        let registry = ToolRegistry::builtins().unwrap();
        let mut step = sql_step();
        step.parameters["shell"] = json!("rm -rf /tmp/example");
        assert!(matches!(
            registry.validate_step(&step),
            Err(AmosError::Validation(_))
        ));

        let mut step = sql_step();
        step.limits.seconds = 301;
        assert!(matches!(
            registry.validate_step(&step),
            Err(AmosError::Validation(_))
        ));
    }

    #[test]
    fn obsolete_contract_only_manifest_cannot_be_dispatched() {
        let manifest = ToolManifest::from_json(include_str!(
            "../tool-packs/templates/spark.dataframe.aggregate.v1.json"
        ))
        .unwrap();
        assert_eq!(manifest.availability, ToolAvailability::ContractOnly);
        let mut registry = ToolRegistry::default();
        registry.register_contract(manifest).unwrap();
        assert!(
            registry
                .implementation("spark.dataframe.aggregate.v1")
                .is_err()
        );
        assert!(
            registry
                .validate_allowed_tools([&"spark.dataframe.aggregate.v1".to_string()])
                .is_err()
        );
    }

    #[test]
    fn duplicate_registration_fails_without_replacing_the_original() {
        let mut registry = ToolRegistry::builtins().unwrap();
        let original = registry.get("sql.readonly.v1").unwrap().clone();
        assert!(matches!(
            registry.register_contract(original.clone()),
            Err(AmosError::Conflict(_))
        ));
        assert_eq!(registry.get("sql.readonly.v1").unwrap(), &original);
    }
}
