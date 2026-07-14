use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use axum::{
    Form, Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Result,
    domain::{Authority, Identity, MemoryObject, MemoryType, ReviewDecision},
    memory::RetrieveQuery,
    runtime::AmosRuntime,
    seed::TENANT,
};

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<AmosRuntime>,
    identities: Arc<BTreeMap<String, Identity>>,
}

pub fn router(runtime: AmosRuntime) -> Router {
    let state = AppState {
        runtime: Arc::new(runtime),
        identities: Arc::new(demo_identities()),
    };
    Router::new()
        .route("/", get(workspace))
        .route("/memory", get(memory_studio))
        .route("/reviews", get(review_queue))
        .route("/operations", get(operations_console))
        .route("/ui/tasks", post(run_task_form))
        .route("/health", get(health))
        .route("/v1/tasks", post(run_task))
        .route("/v1/tasks/{id}", get(get_transaction))
        .route("/v1/transactions/{id}", get(get_transaction))
        .route("/v1/artifacts", get(list_artifacts))
        .route("/v1/artifacts/{id}", get(get_artifact))
        .route("/v1/artifacts/{id}/replay", post(replay))
        .route("/v1/replay/{id}", post(replay))
        .route("/v1/artifacts/{id}/revalidate", post(revalidate))
        .route("/v1/artifacts/{id}/reviews", post(review))
        .route("/v1/reviews", post(review_with_artifact))
        .route("/v1/claims/{id}", get(get_claim))
        .route("/v1/memory", get(list_memory).post(write_memory))
        .route("/v1/memory/search", post(search_memory))
        .route("/v1/memory/{id}/supersede", post(supersede_memory))
        .route("/v1/verify/sql", post(verify_sql))
        .route("/v1/audit", get(audit))
        .route("/v1/jobs", get(list_jobs).post(enqueue_job))
        .route("/v1/connectors/health", get(connector_health))
        .route("/v1/source-events/process", post(process_source_events))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct TaskRequest {
    request: String,
    idempotency_key: Option<String>,
}
async fn run_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TaskRequest>,
) -> Result<Json<crate::domain::RunResult>> {
    let identity = identity(&state, &headers)?;
    let key = request.idempotency_key.unwrap_or_else(|| {
        crate::domain::content_hash(
            &json!({"subject":identity.subject_id,"request":request.request}),
        )
    });
    Ok(Json(
        state
            .runtime
            .run_task(&identity, request.request, key)
            .await?,
    ))
}
#[derive(Debug, Deserialize)]
struct TaskForm {
    request: String,
    identity: Option<String>,
}
async fn run_task_form(
    State(state): State<AppState>,
    Form(form): Form<TaskForm>,
) -> impl IntoResponse {
    let identity = state
        .identities
        .get(form.identity.as_deref().unwrap_or("analyst_001"))
        .cloned()
        .unwrap_or_else(|| state.identities["analyst_001"].clone());
    let key = crate::domain::new_id("ui");
    match state.runtime.run_task(&identity, form.request, key).await {
        Ok(result) => Html(page(
            "Verified analysis",
            &format!(
                "<section class='card'><p class='eyebrow'>Terminal state: {:?}</p><h1>{}</h1><pre>{}</pre><h2>Evidence ledger</h2>{}<form method='post' action='/v1/artifacts/{}/replay'><button>Replay result</button></form></section>",
                result.transaction.state,
                escape(&result.artifact.title),
                escape(&result.artifact.content),
                result
                    .claims
                    .iter()
                    .map(|c| format!(
                        "<article><strong>{}</strong><p>{}</p><small>{:?} · {:?}</small></article>",
                        escape(&c.claim_type),
                        escape(&c.text),
                        c.semantic_validity,
                        c.review_state
                    ))
                    .collect::<String>(),
                result.artifact.artifact_id
            ),
        )),
        Err(error) => Html(page(
            "Analysis failed",
            &format!(
                "<section class='card error'><h1>Analysis did not complete</h1><p>{}</p><a href='/'>Return to workspace</a></section>",
                escape(&error.to_string())
            ),
        )),
    }
}

async fn get_transaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<crate::domain::AnalyticalTransaction>> {
    let user = identity(&state, &headers)?;
    Ok(Json(
        state
            .runtime
            .store
            .get_transaction(&user.tenant_id, &id)?
            .ok_or(crate::error::AmosError::NotFound(id))?,
    ))
}
async fn list_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::domain::Artifact>>> {
    let user = identity(&state, &headers)?;
    Ok(Json(
        state.runtime.store.list_artifacts(&user.tenant_id, 50)?,
    ))
}
async fn get_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = identity(&state, &headers)?;
    let artifact = state
        .runtime
        .store
        .get_artifact(&user.tenant_id, &id)?
        .ok_or_else(|| crate::error::AmosError::NotFound(id.clone()))?;
    let claims = state.runtime.store.list_claims(&user.tenant_id, &id)?;
    let edges = claims
        .iter()
        .flat_map(|claim| {
            state
                .runtime
                .store
                .list_edges_from(&user.tenant_id, "claim", &claim.claim_id)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    Ok(Json(
        json!({"artifact":artifact,"claims":claims,"dependencies":edges}),
    ))
}
async fn replay(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<crate::domain::ReplayResult>> {
    let user = identity(&state, &headers)?;
    Ok(Json(state.runtime.replay(&user, &id)?))
}

async fn revalidate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = identity(&state, &headers)?;
    Ok(Json(state.runtime.revalidate_artifact(&user, &id)?))
}

async fn get_claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let user = identity(&state, &headers)?;
    let claim = state
        .runtime
        .store
        .get_claim(&user.tenant_id, &id)?
        .ok_or_else(|| crate::error::AmosError::NotFound(id.clone()))?;
    let dependencies = state
        .runtime
        .store
        .list_edges_from(&user.tenant_id, "claim", &id)?;
    Ok(Json(json!({"claim":claim,"dependencies":dependencies})))
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    claim_ids: Vec<String>,
    decision: ReviewDecision,
    comment: String,
    correction: Option<Value>,
    authority: Authority,
}
#[derive(Debug, Deserialize)]
struct ReviewWithArtifactRequest {
    artifact_id: String,
    claim_ids: Vec<String>,
    decision: ReviewDecision,
    comment: String,
    correction: Option<Value>,
    authority: Authority,
}
async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ReviewRequest>,
) -> Result<Json<crate::domain::ReviewResult>> {
    let user = identity(&state, &headers)?;
    Ok(Json(
        state
            .runtime
            .review_artifact(
                &user,
                &id,
                input.claim_ids,
                input.decision,
                input.comment,
                input.correction,
                input.authority,
            )
            .await?,
    ))
}
async fn review_with_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ReviewWithArtifactRequest>,
) -> Result<Json<crate::domain::ReviewResult>> {
    let user = identity(&state, &headers)?;
    Ok(Json(
        state
            .runtime
            .review_artifact(
                &user,
                &input.artifact_id,
                input.claim_ids,
                input.decision,
                input.comment,
                input.correction,
                input.authority,
            )
            .await?,
    ))
}
async fn list_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<MemoryObject>>> {
    let user = identity(&state, &headers)?;
    let values = state
        .runtime
        .store
        .list_active_memory(&user.tenant_id)?
        .into_iter()
        .filter(|item| item.permissions.is_subset(&user.permissions))
        .collect();
    Ok(Json(values))
}

#[derive(Debug, Deserialize)]
struct MemorySearchRequest {
    task_text: String,
    required_types: Option<Vec<MemoryType>>,
    time_start: Option<DateTime<Utc>>,
    time_end: Option<DateTime<Utc>>,
    max_items: Option<usize>,
}
async fn search_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<MemorySearchRequest>,
) -> Result<Json<crate::memory::RetrievalResult>> {
    let user = identity(&state, &headers)?;
    let end = input.time_end.unwrap_or_else(Utc::now);
    Ok(Json(
        state.runtime.memory.retrieve(
            &user,
            &RetrieveQuery {
                task_text: input.task_text,
                required_types: input
                    .required_types
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                time_start: input.time_start.unwrap_or(end - Duration::days(365)),
                time_end: end,
                max_items: input.max_items.unwrap_or(20).min(100),
            },
        )?,
    ))
}

#[derive(Debug, Deserialize)]
struct VerifySqlRequest {
    request: String,
    sql: String,
}
async fn verify_sql(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<VerifySqlRequest>,
) -> Result<Json<crate::domain::SqlPreflight>> {
    let user = identity(&state, &headers)?;
    Ok(Json(state.runtime.preflight_sql(
        &user,
        &input.request,
        input.sql,
    )?))
}
async fn write_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(object): Json<MemoryObject>,
) -> Result<(StatusCode, Json<MemoryObject>)> {
    let user = identity(&state, &headers)?;
    state.runtime.memory.write(&user, &object)?;
    Ok((StatusCode::CREATED, Json(object)))
}
async fn supersede_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(object): Json<MemoryObject>,
) -> Result<Json<MemoryObject>> {
    let user = identity(&state, &headers)?;
    Ok(Json(state.runtime.memory.supersede(&user, &id, object)?))
}
#[derive(Deserialize)]
struct Limit {
    limit: Option<usize>,
}
async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Limit>,
) -> Result<Json<Vec<crate::domain::AuditEvent>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(
        state
            .runtime
            .store
            .list_audit(&user.tenant_id, query.limit.unwrap_or(100))?,
    ))
}
async fn connector_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(json!(state.runtime.connector_health().await?)))
}

#[derive(Debug, Deserialize)]
struct JobRequest {
    job_type: String,
    payload: Value,
    idempotency_key: String,
}
async fn enqueue_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<JobRequest>,
) -> Result<(StatusCode, Json<crate::domain::Job>)> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok((
        StatusCode::CREATED,
        Json(state.runtime.scheduler.enqueue(
            &user.tenant_id,
            &input.job_type,
            input.payload,
            input.idempotency_key,
        )?),
    ))
}
async fn list_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Limit>,
) -> Result<Json<Vec<crate::domain::Job>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(
        state
            .runtime
            .store
            .list_jobs(&user.tenant_id, query.limit.unwrap_or(100))?,
    ))
}
#[derive(Debug, Deserialize)]
struct SourceCursor {
    cursor: Option<String>,
}
async fn process_source_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SourceCursor>,
) -> Result<Json<BTreeMap<String, Vec<String>>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(
        state
            .runtime
            .process_source_events(input.cursor.as_deref())
            .await?,
    ))
}
async fn health() -> Json<Value> {
    Json(json!({"status":"ok","runtime":"rust","version":env!("CARGO_PKG_VERSION")}))
}

async fn workspace() -> Html<String> {
    Html(page(
        "AMOS · Verified analysis",
        r#"<section class="hero"><p class="eyebrow">Payment operations workspace</p><h1>Ask the question.<br><em>Trust the answer.</em></h1><p>AMOS verifies the metric, schema, data state, permissions, and support behind every material claim.</p></section><section class="card"><form method="post" action="/ui/tasks"><label for="request">What do you need to know?</label><textarea id="request" name="request" required>Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?</textarea><label for="identity">Identity</label><select id="identity" name="identity"><option value="analyst_001">Maya · Analyst</option><option value="reviewer_001">Noah · Reviewer</option></select><button type="submit">Run verified analysis →</button></form></section><section class="grid"><article><b>01</b><h2>Current definitions</h2><p>Approved metrics and active schemas.</p></article><article><b>02</b><h2>Claim evidence</h2><p>Typed support before publication.</p></article><article><b>03</b><h2>Replayable decisions</h2><p>Recorded inputs, versions, and hashes.</p></article></section>"#,
    ))
}
async fn memory_studio(State(state): State<AppState>) -> Html<String> {
    let identity = &state.identities["admin"];
    let body = state
        .runtime
        .store
        .list_active_memory(&identity.tenant_id)
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            format!(
                "<article><strong>{}</strong><p>{}</p><small>{:?} · {:?} · {}</small></article>",
                escape(&m.logical_key),
                escape(&m.summary),
                m.memory_type,
                m.authority,
                escape(&m.version)
            )
        })
        .collect::<String>();
    Html(page(
        "Memory Studio",
        &format!(
            "<section class='card'><p class='eyebrow'>Memory Studio</p><h1>Governed analytical memory</h1>{body}</section>"
        ),
    ))
}
async fn review_queue(State(state): State<AppState>) -> Html<String> {
    let artifacts = state
        .runtime
        .store
        .list_artifacts(TENANT, 50)
        .unwrap_or_default();
    let body=artifacts.into_iter().map(|a|format!("<article><strong>{}</strong><p>{:?}</p><a href='/v1/artifacts/{}'>Inspect evidence</a></article>",escape(&a.title),a.publication_validity,a.artifact_id)).collect::<String>();
    Html(page(
        "Review Queue",
        &format!(
            "<section class='card'><p class='eyebrow'>Review Queue</p><h1>Human decisions</h1>{body}</section>"
        ),
    ))
}
async fn operations_console(State(state): State<AppState>) -> Html<String> {
    let events = state
        .runtime
        .store
        .list_audit(TENANT, 50)
        .unwrap_or_default();
    let body = events
        .into_iter()
        .map(|e| {
            format!(
                "<article><strong>{}</strong><p>{} · {}</p></article>",
                escape(&e.action),
                escape(&e.actor_id),
                e.created_at
            )
        })
        .collect::<String>();
    Html(page(
        "Operations Console",
        &format!(
            "<section class='card'><p class='eyebrow'>Operations Console</p><h1>Audit and system health</h1>{body}</section>"
        ),
    ))
}

fn identity(state: &AppState, headers: &HeaderMap) -> Result<Identity> {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("analyst_001");
    state
        .identities
        .get(token)
        .cloned()
        .ok_or_else(|| crate::error::AmosError::PermissionDenied("invalid bearer identity".into()))
}
pub fn demo_identities() -> BTreeMap<String, Identity> {
    let base = |subject: &str, roles: &[&str], permissions: &[&str]| Identity {
        tenant_id: TENANT.into(),
        subject_id: subject.into(),
        roles: roles.iter().map(|v| v.to_string()).collect(),
        groups: BTreeSet::new(),
        permissions: permissions.iter().map(|v| v.to_string()).collect(),
        policy_attributes: BTreeMap::new(),
        policy_epoch: 1,
    };
    BTreeMap::from([
        (
            "analyst_001".into(),
            base("analyst_001", &["analyst"], &["analytics", "payments"]),
        ),
        (
            "reviewer_001".into(),
            base("reviewer_001", &["reviewer"], &["analytics", "payments"]),
        ),
        (
            "admin".into(),
            base(
                "admin",
                &["admin", "owner", "reviewer"],
                &["analytics", "payments", "sre", "admin"],
            ),
        ),
    ])
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{}</style></head><body><header><a href="/">AMOS</a><nav><a href="/">Workspace</a><a href="/memory">Memory Studio</a><a href="/reviews">Review Queue</a><a href="/operations">Operations</a></nav></header><main>{}</main></body></html>"#,
        escape(title),
        STYLE,
        body
    )
}
const STYLE: &str = r#":root{font-family:Inter,ui-sans-serif,system-ui;color:#17231c;background:#f6f4ed}*{box-sizing:border-box}body{margin:0}header{height:68px;padding:0 5vw;display:flex;align-items:center;justify-content:space-between;border-bottom:1px solid #d9d9cf}header>a{font:700 22px Georgia;color:#17231c;text-decoration:none}nav{display:flex;gap:18px}nav a{font-size:13px;color:#526159;text-decoration:none}main{width:min(1040px,90vw);margin:60px auto}.hero{max-width:760px}.eyebrow{text-transform:uppercase;letter-spacing:.13em;color:#1f5a3d;font-size:11px;font-weight:800}h1{font:400 clamp(44px,7vw,76px)/1 Georgia;margin:18px 0}h1 em{color:#1f5a3d}.hero>p:last-child{color:#66736d;line-height:1.7;max-width:580px}.card{background:#fffefa;border:1px solid #d9d9cf;border-radius:18px;padding:32px;box-shadow:0 18px 50px #1c2c2214}.card h1{font-size:38px}label{display:block;font-size:12px;font-weight:700;margin:12px 0 7px}textarea,select{width:100%;padding:14px;border:1px solid #d9d9cf;border-radius:10px;background:#faf9f4}textarea{min-height:120px;font:20px Georgia}button{margin-top:16px;padding:13px 18px;border:0;border-radius:10px;background:#17231c;color:#fff;font-weight:700}.grid{display:grid;grid-template-columns:repeat(3,1fr);margin-top:35px;border-top:1px solid #d9d9cf}.grid article{padding:26px}.grid article+article{border-left:1px solid #d9d9cf}article{padding:14px 0;border-bottom:1px solid #e7e6de}article p,small{color:#66736d}pre{white-space:pre-wrap;line-height:1.6}.error{border-color:#a86666}@media(max-width:700px){nav{display:none}.grid{grid-template-columns:1fr}.grid article+article{border-left:0;border-top:1px solid #d9d9cf}}"#;
