use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AmosError, Result,
    domain::{
        AnalyticalTransaction, Artifact, Claim, ContextManifest, DependencyEdge, EdgeEndpoint,
        ExecutionRecord, PolicyVisibility, PublicationValidity, ReplayAvailability, ReviewState,
        SemanticValidity, SupersessionState, TypedPlan, VerificationRecord, content_hash,
        stable_id,
    },
    model::{AnalysisKind, NARRATIVE_SCHEMA_VERSION, NarrativePlan},
    packs::AnalysisPack,
    workers::ChartWorker,
};

pub const FACT_METRIC_CHANGE: &str = "metric.rate_change";
pub const FACT_CONCENTRATION_TOP: &str = "concentration.top";
pub const FACT_TREND_DAILY: &str = "trend.daily";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VerifiedFact {
    pub fact_id: String,
    pub claim_type: String,
    pub canonical_text: String,
    pub payload: Value,
    pub supporting_execution_ids: Vec<String>,
    pub supporting_verification_ids: Vec<String>,
    pub governed_memory_ids: Vec<String>,
    pub source_versions: BTreeMap<String, String>,
    pub review_required: bool,
    pub freshness_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VerifiedFactCatalog {
    pub catalog_id: String,
    pub tenant_id: String,
    pub atxn_id: String,
    pub facts: Vec<VerifiedFact>,
    pub content_hash: String,
}

impl VerifiedFactCatalog {
    pub fn fact(&self, fact_id: &str) -> Result<&VerifiedFact> {
        self.facts
            .iter()
            .find(|fact| fact.fact_id == fact_id)
            .ok_or_else(|| AmosError::Validation(format!("unknown verified fact {fact_id}")))
    }
}

pub fn build_fact_catalog(
    atxn: &AnalyticalTransaction,
    manifest: &ContextManifest,
    plan: &TypedPlan,
    executions: &[ExecutionRecord],
    verifications: &[VerificationRecord],
    pack: &AnalysisPack,
) -> Result<VerifiedFactCatalog> {
    let rate_fields = &pack.verifier_profile.rate_comparison;
    let summary = execution(executions, "summary")?;
    let summary_rows = rows(summary, "rate comparison")?;
    let current = row_with_label(
        summary_rows,
        &rate_fields.period_field,
        &rate_fields.current_label,
        "current rate",
    )?;
    let baseline = row_with_label(
        summary_rows,
        &rate_fields.period_field,
        &rate_fields.baseline_label,
        "baseline rate",
    )?;
    let current_rate = checked_rate(
        current,
        &rate_fields.numerator_field,
        &rate_fields.denominator_field,
        &rate_fields.rate_field,
    )?;
    let baseline_rate = checked_rate(
        baseline,
        &rate_fields.numerator_field,
        &rate_fields.denominator_field,
        &rate_fields.rate_field,
    )?;
    let change = current_rate - baseline_rate;

    let concentration_execution = execution(executions, "concentration")?;
    let top = rows(concentration_execution, "concentration")?
        .first()
        .ok_or_else(|| AmosError::Execution("concentration output is empty".into()))?;
    let concentration_fields = &pack.verifier_profile.concentration;
    let concentration_rate = checked_rate(
        top,
        &concentration_fields.numerator_field,
        &concentration_fields.denominator_field,
        &concentration_fields.rate_field,
    )?;
    let concentration_dimensions = pack
        .result_schemas
        .get(&AnalysisKind::Concentration)
        .ok_or_else(|| {
            AmosError::Validation("analysis pack has no concentration result schema".into())
        })?
        .iter()
        .filter(|column| {
            *column != &concentration_fields.numerator_field
                && *column != &concentration_fields.denominator_field
                && *column != &concentration_fields.rate_field
        })
        .map(|column| scalar_text(top, column, "concentration"))
        .collect::<Result<Vec<_>>>()?;
    let concentration_label = concentration_dimensions.join(" / ");
    let concentration_numerator =
        required_u64(top, &concentration_fields.numerator_field, "concentration")?;
    let concentration_denominator = required_u64(
        top,
        &concentration_fields.denominator_field,
        "concentration",
    )?;

    let timeseries_execution = execution(executions, "timeseries")?;
    let trend_rows = rows(timeseries_execution, "timeseries")?;
    let trend_fields = &pack.verifier_profile.timeseries;
    let points = trend_rows
        .iter()
        .map(|row| {
            Ok(json!({
                "label": required_text(row, &trend_fields.label_field, "timeseries")?,
                "value": required_f64(row, &trend_fields.value_field, "timeseries")?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;

    let metric_memory_id = role_id(manifest, "metric_definition")?;
    let schema_id = role_id(manifest, "active_schema")?;
    let snapshot_id = role_id(manifest, "data_snapshot")?;
    let metric_object = manifest
        .selected_objects
        .iter()
        .find(|object| object.object_id == metric_memory_id)
        .ok_or_else(|| {
            AmosError::Validation("context manifest lacks the governed metric definition".into())
        })?;
    let metric_name = metric_object
        .content
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| {
            AmosError::Validation("governed metric definition has no metric name".into())
        })?;
    let metric_version = metric_object
        .content
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or(&metric_object.source_version);
    let metric_id = format!("{metric_name}:{metric_version}");
    let governed_memory_ids = vec![metric_memory_id, schema_id, snapshot_id];
    let freshness_labels = manifest
        .selected_objects
        .iter()
        .filter_map(|object| {
            object
                .content
                .get("freshness_warning")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    let metric_text = if change == 0.0 {
        format!(
            "Governed metric {metric_name} remained at {} across the governed periods.",
            percent(current_rate)
        )
    } else {
        format!(
            "Governed metric {metric_name} {} from {} to {} ({} percentage points).",
            if change > 0.0 {
                "increased"
            } else {
                "decreased"
            },
            percent(baseline_rate),
            percent(current_rate),
            percentage_points(change)
        )
    };
    let facts = vec![
        VerifiedFact {
            fact_id: FACT_METRIC_CHANGE.into(),
            claim_type: "metric_comparison".into(),
            canonical_text: metric_text,
            payload: json!({
                "metric_id":metric_id,
                "current_value":current_rate,
                "baseline_value":baseline_rate,
                "absolute_change":change
            }),
            supporting_execution_ids: vec![summary.execution_id.clone()],
            supporting_verification_ids: verification_ids_for_step(plan, "summary", verifications)?,
            governed_memory_ids: governed_memory_ids.clone(),
            source_versions: manifest.source_versions.clone(),
            review_required: false,
            freshness_labels: freshness_labels.clone(),
        },
        VerifiedFact {
            fact_id: FACT_CONCENTRATION_TOP.into(),
            claim_type: "concentration".into(),
            canonical_text: format!(
                "The largest verified concentration is {concentration_label}: {concentration_numerator} of {concentration_denominator} ({}).",
                percent(concentration_rate)
            ),
            payload: top.clone(),
            supporting_execution_ids: vec![concentration_execution.execution_id.clone()],
            supporting_verification_ids: verification_ids_for_step(
                plan,
                "concentration",
                verifications,
            )?,
            governed_memory_ids: governed_memory_ids.clone(),
            source_versions: manifest.source_versions.clone(),
            review_required: false,
            freshness_labels: freshness_labels.clone(),
        },
        VerifiedFact {
            fact_id: FACT_TREND_DAILY.into(),
            claim_type: "timeseries".into(),
            canonical_text: format!(
                "The governed trend contains {} verified aggregate points.",
                points.len()
            ),
            payload: json!({
                "accessible_label":trend_fields.accessible_label,
                "points":points
            }),
            supporting_execution_ids: vec![timeseries_execution.execution_id.clone()],
            supporting_verification_ids: verification_ids_for_step(
                plan,
                "timeseries",
                verifications,
            )?,
            governed_memory_ids,
            source_versions: manifest.source_versions.clone(),
            review_required: false,
            freshness_labels,
        },
    ];
    let hash = content_hash(&facts)?;
    Ok(VerifiedFactCatalog {
        catalog_id: stable_id("facts", &(&atxn.tenant_id, &atxn.atxn_id, &hash))?,
        tenant_id: atxn.tenant_id.clone(),
        atxn_id: atxn.atxn_id.clone(),
        facts,
        content_hash: hash,
    })
}

pub fn validate_narrative_plan(
    narrative: &NarrativePlan,
    catalog: &VerifiedFactCatalog,
    permitted_memory_ids: &BTreeSet<String>,
    pack: &AnalysisPack,
) -> Result<()> {
    narrative.validate_shape()?;
    if narrative.schema_version != NARRATIVE_SCHEMA_VERSION {
        return Err(AmosError::Validation(
            "narrative schema version is unsupported".into(),
        ));
    }
    let fact_ids = catalog
        .facts
        .iter()
        .map(|fact| fact.fact_id.as_str())
        .collect::<BTreeSet<_>>();
    let ordered = narrative
        .finding_order
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if ordered != fact_ids || ordered.len() != narrative.finding_order.len() {
        return Err(AmosError::Validation(
            "narrative finding order must reference every verified fact exactly once".into(),
        ));
    }
    validate_model_text(&narrative.title, &fact_ids)?;
    validate_model_text(&narrative.executive_summary, &fact_ids)?;
    for section in &narrative.sections {
        validate_model_text(&section.heading, &fact_ids)?;
        validate_model_text(&section.commentary, &fact_ids)?;
        validate_fact_ids(&section.fact_ids, &fact_ids)?;
    }
    for claim in &narrative.judgment_claims {
        if !pack.claim_types.contains(&claim.claim_type)
            || claim.support_fact_ids.is_empty()
            || claim
                .support_memory_ids
                .iter()
                .any(|id| !permitted_memory_ids.contains(id))
            || (pack
                .review_triggering_claim_types
                .contains(&claim.claim_type)
                && !claim.review_required)
        {
            return Err(AmosError::Validation(
                "narrative judgment crosses a fact, memory, claim, or review boundary".into(),
            ));
        }
        validate_model_text(&claim.text, &fact_ids)?;
        validate_fact_ids(&claim.support_fact_ids, &fact_ids)?;
    }
    for slide in &narrative.slide_outline {
        validate_model_text(&slide.title, &fact_ids)?;
        validate_fact_ids(&slide.fact_ids, &fact_ids)?;
    }
    Ok(())
}

pub fn compile_artifact(
    atxn: &AnalyticalTransaction,
    manifest: &ContextManifest,
    catalog: &VerifiedFactCatalog,
    narrative: &NarrativePlan,
    pack: &AnalysisPack,
    model_identity: &str,
) -> Result<(Artifact, Vec<Claim>, Vec<DependencyEdge>)> {
    let timeseries = catalog.fact(FACT_TREND_DAILY)?;
    let points = timeseries
        .payload
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| AmosError::Validation("timeseries fact has no points".into()))?
        .iter()
        .map(|point| {
            Ok((
                required_text(point, "label", "fact point")?.to_string(),
                required_f64(point, "value", "fact point")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let (svg, chart_hash) = ChartWorker
        .timeseries_svg_with_label(&points, &pack.verifier_profile.timeseries.accessible_label)?;
    let executive_summary = expand_fact_placeholders(&narrative.executive_summary, catalog)?;
    let sensitivity = manifest
        .selected_objects
        .iter()
        .map(|object| object.sensitivity.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    let freshness = catalog
        .facts
        .iter()
        .flat_map(|fact| fact.freshness_labels.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("; ");
    let mut html = format!(
        "<article class=\"analysis\"><header><p class=\"eyebrow\">Verified AMOS analysis</p><h1>{}</h1><p>{}</p><dl class=\"facts\"><div><dt>Sensitivity</dt><dd>{}</dd></div><div><dt>Audience</dt><dd>{}</dd></div><div><dt>Source freshness</dt><dd>{}</dd></div><div><dt>Model identity</dt><dd>{}</dd></div><div><dt>Publication state</dt><dd>Draft — review required</dd></div></dl></header><figure>{}<figcaption>{}</figcaption></figure><p class=\"hash\">Chart hash: {}</p><section><h2>Verified findings</h2><ol>",
        escape_html(&narrative.title),
        escape_html(&executive_summary),
        escape_html(&sensitivity),
        escape_html(&pack.audience),
        escape_html(&freshness),
        escape_html(model_identity),
        svg,
        escape_html(&pack.verifier_profile.timeseries.accessible_label),
        escape_html(&chart_hash),
    );
    for fact_id in &narrative.finding_order {
        let fact = catalog.fact(fact_id)?;
        html.push_str(&format!(
            "<li id=\"fact-{}\">{}</li>",
            escape_html(&fact.fact_id),
            escape_html(&fact.canonical_text)
        ));
    }
    html.push_str("</ol></section>");
    for section in &narrative.sections {
        html.push_str(&format!(
            "<section><h2>{}</h2><p>{}</p><ul>",
            escape_html(&section.heading),
            escape_html(&expand_fact_placeholders(&section.commentary, catalog)?)
        ));
        for fact_id in &section.fact_ids {
            html.push_str(&format!(
                "<li>{}</li>",
                escape_html(&catalog.fact(fact_id)?.canonical_text)
            ));
        }
        html.push_str("</ul></section>");
    }
    if !narrative.judgment_claims.is_empty() {
        html.push_str("<section><h2>Judgment requiring review</h2><ul>");
        for judgment in &narrative.judgment_claims {
            html.push_str(&format!(
                "<li>{}</li>",
                escape_html(&expand_fact_placeholders(&judgment.text, catalog)?)
            ));
        }
        html.push_str("</ul></section>");
    }
    html.push_str("</article>");

    let artifact_id = stable_id(
        "art",
        &(
            &atxn.tenant_id,
            &atxn.atxn_id,
            &catalog.content_hash,
            narrative,
            &chart_hash,
        ),
    )?;
    let artifact = Artifact {
        tenant_id: atxn.tenant_id.clone(),
        artifact_id: artifact_id.clone(),
        atxn_id: atxn.atxn_id.clone(),
        artifact_type: "html_report".into(),
        title: narrative.title.clone(),
        content: html.clone(),
        content_hash: content_hash(&html)?,
        audience: pack.audience.clone(),
        risk_class: pack.risk_class,
        object_state: "finalized".into(),
        publication_validity: PublicationValidity::Draft,
        created_at: atxn.created_at,
    };

    let mut claims = Vec::new();
    let mut edges = Vec::new();
    for fact in &catalog.facts {
        let claim = fact_claim(&artifact, fact)?;
        append_fact_edges(atxn, manifest, &claim, fact, &mut edges)?;
        claims.push(claim);
    }
    for (index, judgment) in narrative.judgment_claims.iter().enumerate() {
        let supporting_facts = judgment
            .support_fact_ids
            .iter()
            .map(|id| catalog.fact(id))
            .collect::<Result<Vec<_>>>()?;
        let support_execution_ids = supporting_facts
            .iter()
            .flat_map(|fact| fact.supporting_execution_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let verification_ids = supporting_facts
            .iter()
            .flat_map(|fact| fact.supporting_verification_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let text = expand_fact_placeholders(&judgment.text, catalog)?;
        let claim = Claim {
            tenant_id: artifact.tenant_id.clone(),
            claim_id: stable_id(
                "clm",
                &(
                    &artifact.tenant_id,
                    &artifact.artifact_id,
                    &judgment.claim_type,
                    index,
                ),
            )?,
            artifact_id: artifact.artifact_id.clone(),
            claim_type: judgment.claim_type.clone(),
            text,
            payload: json!({
                "review_required":judgment.review_required,
                "support_fact_ids":judgment.support_fact_ids,
                "support_memory_ids":judgment.support_memory_ids,
            }),
            risk_class: artifact.risk_class,
            support_execution_ids,
            verification_ids,
            publication_validity: PublicationValidity::Draft,
            semantic_validity: SemanticValidity::Current,
            policy_visibility: PolicyVisibility::Allowed,
            replay_availability: ReplayAvailability::Level3,
            review_state: ReviewState::NeedsReview,
            supersession_state: SupersessionState::Active,
        };
        for fact in supporting_facts {
            append_fact_edges(atxn, manifest, &claim, fact, &mut edges)?;
        }
        for memory_id in &judgment.support_memory_ids {
            edges.push(dependency_edge(
                atxn,
                &claim.claim_id,
                "supported_by_memory",
                "memory",
                memory_id,
                source_version_for_memory(manifest, memory_id),
            )?);
        }
        claims.push(claim);
    }
    edges.sort_by(|left, right| left.edge_id.cmp(&right.edge_id));
    edges.dedup_by(|left, right| left.edge_id == right.edge_id);
    Ok((artifact, claims, edges))
}

fn fact_claim(artifact: &Artifact, fact: &VerifiedFact) -> Result<Claim> {
    Ok(Claim {
        tenant_id: artifact.tenant_id.clone(),
        claim_id: stable_id(
            "clm",
            &(&artifact.tenant_id, &artifact.artifact_id, &fact.fact_id),
        )?,
        artifact_id: artifact.artifact_id.clone(),
        claim_type: fact.claim_type.clone(),
        text: fact.canonical_text.clone(),
        payload: fact.payload.clone(),
        risk_class: artifact.risk_class,
        support_execution_ids: fact.supporting_execution_ids.clone(),
        verification_ids: fact.supporting_verification_ids.clone(),
        publication_validity: PublicationValidity::Draft,
        semantic_validity: SemanticValidity::Current,
        policy_visibility: PolicyVisibility::Allowed,
        replay_availability: ReplayAvailability::Level3,
        review_state: if fact.review_required {
            ReviewState::NeedsReview
        } else {
            ReviewState::Verified
        },
        supersession_state: SupersessionState::Active,
    })
}

fn append_fact_edges(
    atxn: &AnalyticalTransaction,
    manifest: &ContextManifest,
    claim: &Claim,
    fact: &VerifiedFact,
    edges: &mut Vec<DependencyEdge>,
) -> Result<()> {
    for execution_id in &fact.supporting_execution_ids {
        edges.push(dependency_edge(
            atxn,
            &claim.claim_id,
            "computed_by",
            "execution",
            execution_id,
            None,
        )?);
    }
    for verification_id in &fact.supporting_verification_ids {
        edges.push(dependency_edge(
            atxn,
            &claim.claim_id,
            "verified_by",
            "verification",
            verification_id,
            None,
        )?);
    }
    let metric = role_id(manifest, "metric_definition")?;
    let schema = role_id(manifest, "active_schema")?;
    let snapshot = role_id(manifest, "data_snapshot")?;
    for (relation, memory_id) in [
        ("governed_by_metric", metric),
        ("governed_by_schema", schema),
        ("scoped_to_data_state", snapshot),
    ] {
        edges.push(dependency_edge(
            atxn,
            &claim.claim_id,
            relation,
            "memory",
            &memory_id,
            source_version_for_memory(manifest, &memory_id),
        )?);
    }
    Ok(())
}

fn dependency_edge(
    atxn: &AnalyticalTransaction,
    claim_id: &str,
    relation: &str,
    target_type: &str,
    target_id: &str,
    source_version: Option<String>,
) -> Result<DependencyEdge> {
    let mut edge = DependencyEdge {
        edge_id: stable_id(
            "edge",
            &(
                &atxn.tenant_id,
                &atxn.atxn_id,
                claim_id,
                relation,
                target_type,
                target_id,
            ),
        )?,
        tenant_id: atxn.tenant_id.clone(),
        from: EdgeEndpoint {
            endpoint_type: "claim".into(),
            id: claim_id.into(),
        },
        relation: relation.into(),
        to: EdgeEndpoint {
            endpoint_type: target_type.into(),
            id: target_id.into(),
        },
        source_version,
        created_by_atxn: atxn.atxn_id.clone(),
        content_hash: String::new(),
    };
    edge.content_hash = content_hash(&edge)?;
    Ok(edge)
}

fn validate_model_text(text: &str, fact_ids: &BTreeSet<&str>) -> Result<()> {
    let mut remainder = text;
    let mut free_text = String::new();
    while let Some(start) = remainder.find("{{fact:") {
        free_text.push_str(&remainder[..start]);
        let after = &remainder[start + 7..];
        let end = after.find("}}").ok_or_else(|| {
            AmosError::Validation("narrative contains an unterminated fact placeholder".into())
        })?;
        let fact_id = &after[..end];
        if !fact_ids.contains(fact_id) {
            return Err(AmosError::Validation(format!(
                "narrative references unknown fact {fact_id}"
            )));
        }
        remainder = &after[end + 2..];
    }
    free_text.push_str(remainder);
    if free_text.contains("{{")
        || free_text
            .chars()
            .any(|character| character.is_ascii_digit())
    {
        return Err(AmosError::Validation(
            "model-authored text contains an unbound placeholder or numeric literal".into(),
        ));
    }
    Ok(())
}

fn validate_fact_ids(ids: &[String], known: &BTreeSet<&str>) -> Result<()> {
    if ids.is_empty() || ids.iter().any(|id| !known.contains(id.as_str())) {
        return Err(AmosError::Validation(
            "narrative contains an empty or unknown fact reference".into(),
        ));
    }
    Ok(())
}

fn expand_fact_placeholders(text: &str, catalog: &VerifiedFactCatalog) -> Result<String> {
    let mut expanded = text.to_string();
    for fact in &catalog.facts {
        expanded = expanded.replace(
            &format!("{{{{fact:{}}}}}", fact.fact_id),
            &fact.canonical_text,
        );
    }
    if expanded.contains("{{fact:") {
        return Err(AmosError::Validation(
            "narrative contains an unresolved fact placeholder".into(),
        ));
    }
    Ok(expanded)
}

fn verification_ids_for_step(
    plan: &TypedPlan,
    step_id: &str,
    verifications: &[VerificationRecord],
) -> Result<Vec<String>> {
    let step = plan
        .steps
        .iter()
        .find(|step| step.step_id == step_id)
        .ok_or_else(|| AmosError::NotFound(format!("plan step {step_id}")))?;
    let input_hash = content_hash(step)?;
    let ids = verifications
        .iter()
        .filter(|verification| {
            verification.profile_version == 1
                && verification.input_hash == input_hash
                && verification.outcome != crate::domain::Outcome::Reject
        })
        .map(|verification| verification.verification_id.clone())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(AmosError::Validation(format!(
            "plan step {step_id} has no passing verifier record"
        )));
    }
    Ok(ids)
}

fn checked_rate(
    row: &Value,
    numerator_field: &str,
    denominator_field: &str,
    rate_field: &str,
) -> Result<f64> {
    let numerator = required_u64(row, numerator_field, "rate row")?;
    let denominator = required_u64(row, denominator_field, "rate row")?;
    if denominator == 0 || numerator > denominator {
        return Err(AmosError::Validation(
            "verified rate has an invalid numerator or denominator".into(),
        ));
    }
    let recomputed = numerator as f64 / denominator as f64;
    let reported = required_f64(row, rate_field, "rate row")?;
    if (reported - recomputed).abs() > 1e-12 {
        return Err(AmosError::Validation(
            "reported rate does not recompute from verified aggregates".into(),
        ));
    }
    Ok(recomputed)
}

fn rows<'a>(execution: &'a ExecutionRecord, context: &str) -> Result<&'a [Value]> {
    execution
        .output
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| AmosError::Execution(format!("{context} output is not rows")))
}

fn execution<'a>(executions: &'a [ExecutionRecord], step_id: &str) -> Result<&'a ExecutionRecord> {
    executions
        .iter()
        .find(|execution| execution.step_id == step_id)
        .ok_or_else(|| AmosError::NotFound(format!("execution for step {step_id}")))
}

fn row_with_label<'a>(
    rows: &'a [Value],
    field: &str,
    label: &str,
    context: &str,
) -> Result<&'a Value> {
    rows.iter()
        .find(|row| row.get(field).and_then(Value::as_str) == Some(label))
        .ok_or_else(|| AmosError::Execution(format!("{context} row is missing")))
}

fn role_id(manifest: &ContextManifest, role: &str) -> Result<String> {
    manifest
        .required_role_coverage
        .get(role)
        .and_then(|ids| ids.first())
        .cloned()
        .ok_or_else(|| AmosError::RequiredRoleMissing(role.into()))
}

fn source_version_for_memory(manifest: &ContextManifest, memory_id: &str) -> Option<String> {
    manifest
        .selected_objects
        .iter()
        .find(|object| object.object_id == memory_id)
        .map(|object| object.source_version.clone())
}

fn required_u64(value: &Value, field: &str, context: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| AmosError::Execution(format!("{context} has no integer {field}")))
}

fn required_f64(value: &Value, field: &str, context: &str) -> Result<f64> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| AmosError::Execution(format!("{context} has no finite {field}")))
}

fn required_text<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| AmosError::Execution(format!("{context} has no text {field}")))
}

fn scalar_text(value: &Value, field: &str, context: &str) -> Result<String> {
    match value.get(field) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        Some(Value::Number(number)) => Ok(number.to_string()),
        Some(Value::Bool(boolean)) => Ok(boolean.to_string()),
        _ => Err(AmosError::Execution(format!(
            "{context} has no scalar dimension value {field}"
        ))),
    }
}

fn percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn percentage_points(value: f64) -> String {
    format!("{:+.1}", value * 100.0)
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrative_rejects_unknown_facts_and_unbound_numbers() {
        let catalog = VerifiedFactCatalog {
            catalog_id: "facts".into(),
            tenant_id: "tenant".into(),
            atxn_id: "atxn".into(),
            facts: vec![VerifiedFact {
                fact_id: FACT_METRIC_CHANGE.into(),
                claim_type: "metric_comparison".into(),
                canonical_text: "Verified value is 5.4%.".into(),
                payload: json!({}),
                supporting_execution_ids: vec!["exec".into()],
                supporting_verification_ids: vec!["ver".into()],
                governed_memory_ids: vec!["memory".into()],
                source_versions: BTreeMap::new(),
                review_required: false,
                freshness_labels: vec![],
            }],
            content_hash: "hash".into(),
        };
        let facts = BTreeSet::from([FACT_METRIC_CHANGE]);
        assert!(validate_model_text("A value is 5.4%.", &facts).is_err());
        assert!(validate_model_text("{{fact:unknown}}", &facts).is_err());
        assert!(validate_model_text("The result is {{fact:metric.rate_change}}.", &facts).is_ok());
        assert_eq!(
            expand_fact_placeholders("Result: {{fact:metric.rate_change}}", &catalog)
                .expect("placeholder"),
            "Result: Verified value is 5.4%."
        );
    }

    #[test]
    fn html_escape_neutralizes_model_markup() {
        assert_eq!(
            escape_html("<script>alert('x')</script>"),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"
        );
    }

    #[test]
    fn narrative_plan_rejects_unknown_evidence_and_numeric_literals() {
        let catalog = VerifiedFactCatalog {
            catalog_id: "facts".into(),
            tenant_id: "tenant".into(),
            atxn_id: "atxn".into(),
            facts: vec![VerifiedFact {
                fact_id: FACT_METRIC_CHANGE.into(),
                claim_type: "metric_comparison".into(),
                canonical_text: "Verified value is 5.4%.".into(),
                payload: json!({}),
                supporting_execution_ids: vec!["exec".into()],
                supporting_verification_ids: vec!["ver".into()],
                governed_memory_ids: vec!["memory".into()],
                source_versions: BTreeMap::new(),
                review_required: false,
                freshness_labels: vec![],
            }],
            content_hash: "hash".into(),
        };
        let pack = AnalysisPack::subscription_churn().expect("pack");
        let mut narrative = NarrativePlan {
            schema_version: NARRATIVE_SCHEMA_VERSION.into(),
            title: "Churn review".into(),
            executive_summary: "{{fact:metric.rate_change}}".into(),
            finding_order: vec![FACT_METRIC_CHANGE.into()],
            sections: vec![crate::model::NarrativeSection {
                heading: "Finding".into(),
                fact_ids: vec![FACT_METRIC_CHANGE.into()],
                commentary: "The verified movement merits review.".into(),
            }],
            judgment_claims: vec![],
            slide_outline: vec![crate::model::NarrativeSlide {
                title: "Verified churn".into(),
                fact_ids: vec![FACT_METRIC_CHANGE.into()],
            }],
        };
        let memory_ids = BTreeSet::from(["memory".into()]);
        validate_narrative_plan(&narrative, &catalog, &memory_ids, &pack).expect("valid narrative");

        narrative.executive_summary = "Churn reached 5.4%.".into();
        assert!(validate_narrative_plan(&narrative, &catalog, &memory_ids, &pack).is_err());
        narrative.executive_summary = "{{fact:metric.rate_change}}".into();
        narrative.judgment_claims = vec![crate::model::NarrativeJudgmentClaim {
            claim_type: "causal".into(),
            text: "A campaign may have contributed.".into(),
            support_fact_ids: vec![FACT_METRIC_CHANGE.into()],
            support_memory_ids: vec!["unknown-memory".into()],
            review_required: true,
        }];
        assert!(validate_narrative_plan(&narrative, &catalog, &memory_ids, &pack).is_err());
    }
}
