use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Form, Json, Router,
    extract::{DefaultBodyLimit, Extension, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, X_CONTENT_TYPE_OPTIONS},
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Result,
    auth::{IdentityProvider, StaticIdentityProvider},
    domain::{
        Authority, ErasureReceipt, Identity, MemoryObject, MemoryType, RetentionCommand,
        RetentionRecord, ReviewDecision,
    },
    error::AmosError,
    memory::RetrieveQuery,
    runtime::AmosRuntime,
};

pub use crate::auth::demo_identities;

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<AmosRuntime>,
    identity_provider: Arc<dyn IdentityProvider>,
}

pub fn router(runtime: AmosRuntime, identity_provider: Arc<dyn IdentityProvider>) -> Router {
    let state = AppState {
        runtime: Arc::new(runtime),
        identity_provider,
    };
    let protected = Router::new()
        .route("/", get(workspace))
        .route("/memory", get(memory_studio))
        .route("/reviews", get(review_queue))
        .route("/operations", get(operations_console))
        .route("/ui/tasks", post(run_task_form))
        .route("/v1/tasks", post(run_task))
        .route("/v1/tasks/{id}", get(get_transaction))
        .route("/v1/transactions/{id}", get(get_transaction))
        .route("/v1/artifacts", get(list_artifacts))
        .route("/v1/artifacts/page", get(list_artifacts_page))
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
        .route("/v1/metrics", get(metrics))
        .route("/v1/retention", post(set_retention))
        .route("/v1/retention/memory/{id}/erase", post(erase_memory))
        .route("/v1/jobs", get(list_jobs).post(enqueue_job))
        .route("/v1/jobs/process", post(process_jobs))
        .route("/v1/outbox", get(list_outbox).post(drain_outbox))
        .route("/v1/connectors/health", get(connector_health))
        .route("/v1/source-events/process", post(process_source_events))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new()
        .route("/health", get(health))
        .route("/v1/openapi.json", get(openapi))
        .merge(protected)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(request_controls))
}

pub fn demo_router(runtime: AmosRuntime) -> Router {
    router(runtime, Arc::new(StaticIdentityProvider::demo()))
}

async fn request_controls(mut request: Request, next: Next) -> Response {
    let request_id = safe_request_id(request.headers().get("x-request-id"))
        .unwrap_or_else(|| crate::domain::new_id("req"));
    let correlation_id = safe_request_id(request.headers().get("x-correlation-id"))
        .unwrap_or_else(|| request_id.clone());
    request.extensions_mut().insert(RequestIds {
        request_id: request_id.clone(),
        correlation_id: correlation_id.clone(),
    });
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if !headers.contains_key("x-request-id")
        && let Ok(value) = HeaderValue::from_str(&request_id)
    {
        headers.insert(HeaderName::from_static("x-request-id"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&correlation_id) {
        headers.insert(HeaderName::from_static("x-correlation-id"), value);
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Clone)]
struct RequestIds {
    request_id: String,
    correlation_id: String,
}

fn safe_request_id(value: Option<&HeaderValue>) -> Option<String> {
    let value = value?.to_str().ok()?;
    (value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| value.to_string())
}

async fn authenticate(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    match bearer_token(request.headers())
        .and_then(|token| state.identity_provider.authenticate_bearer(token))
    {
        Ok(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str> {
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| AmosError::Unauthenticated("bearer credentials are required".into()))?
        .to_str()
        .map_err(|_| AmosError::Unauthenticated("authorization header is invalid".into()))?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next().unwrap_or_default();
    let token = parts.next().unwrap_or_default();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || parts.next().is_some() {
        return Err(AmosError::Unauthenticated(
            "authorization header must contain one bearer token".into(),
        ));
    }
    Ok(token)
}

#[derive(Debug, Deserialize)]
struct TaskRequest {
    request: String,
    idempotency_key: String,
}
async fn run_task(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Extension(request_ids): Extension<RequestIds>,
    Json(request): Json<TaskRequest>,
) -> Result<Json<crate::domain::RunResult>> {
    tracing::info!(
        request_id = %request_ids.request_id,
        correlation_id = %request_ids.correlation_id,
        tenant_id = %identity.tenant_id,
        subject_id = %identity.subject_id,
        "task request admitted"
    );
    Ok(Json(
        state
            .runtime
            .run_task(&identity, request.request, request.idempotency_key)
            .await?,
    ))
}
#[derive(Debug, Deserialize)]
struct TaskForm {
    request: String,
}
async fn run_task_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<TaskForm>,
) -> Result<Html<String>> {
    let key = crate::domain::new_id("ui");
    let result = state.runtime.run_task(&identity, form.request, key).await?;
    Ok(Html(page(
        "Verified analysis",
        &format!(
            "<section class='card'><p class='eyebrow'>Terminal state: {:?}</p><h1>{}</h1><pre>{}</pre><h2>Evidence ledger</h2>{}<form method='post' action='/v1/artifacts/{}/replay'><button>Replay result</button></form></section>",
            result.transaction.state,
            escape(&result.artifact.title),
            escape(&result.artifact.content),
            result
                .claims
                .iter()
                .map(|claim| format!(
                    "<article><strong>{}</strong><p>{}</p><small>{:?} · {:?}</small></article>",
                    escape(&claim.claim_type),
                    escape(&claim.text),
                    claim.semantic_validity,
                    claim.review_state
                ))
                .collect::<String>(),
            result.artifact.artifact_id
        ),
    )))
}

async fn get_transaction(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<crate::domain::AnalyticalTransaction>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.get_transaction_for(&user, &id))
            .await?,
    ))
}
async fn list_artifacts(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
) -> Result<Json<Vec<crate::domain::Artifact>>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.list_artifacts_for(&user, 50))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct CursorQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

async fn list_artifacts_page(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Query(query): Query<CursorQuery>,
) -> Result<Json<crate::connectors::Page<crate::domain::Artifact>>> {
    let after = query
        .cursor
        .as_deref()
        .map(decode_artifact_cursor)
        .transpose()?;
    let limit = query.limit.unwrap_or(50);
    let mut page = state
        .runtime
        .execute_blocking(move |runtime| {
            runtime.list_artifacts_page_for(&user, after.as_deref(), limit)
        })
        .await?;
    page.next_cursor = page
        .next_cursor
        .map(|artifact_id| encode_artifact_cursor(&artifact_id));
    Ok(Json(page))
}

fn encode_artifact_cursor(artifact_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("artifact:{artifact_id}"))
}

fn decode_artifact_cursor(cursor: &str) -> Result<String> {
    let decoded = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| AmosError::Validation("artifact cursor is malformed".into()))?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| AmosError::Validation("artifact cursor is not UTF-8".into()))?;
    decoded
        .strip_prefix("artifact:")
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AmosError::Validation("artifact cursor has the wrong resource type".into()))
}
async fn get_artifact(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let (artifact, claims, edges) = state
        .runtime
        .execute_blocking(move |runtime| runtime.get_artifact_for(&user, &id))
        .await?;
    Ok(Json(
        json!({"artifact":artifact,"claims":claims,"dependencies":edges}),
    ))
}

#[derive(Debug, Deserialize)]
struct ReplayRequest {
    idempotency_key: String,
}

async fn replay(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
    request: Option<Json<ReplayRequest>>,
) -> Result<Json<crate::domain::ReplayResult>> {
    let idempotency_key = request
        .map(|Json(request)| request.idempotency_key)
        .unwrap_or_default();
    Ok(Json(
        state
            .runtime
            .replay_async(&user, id, idempotency_key)
            .await?,
    ))
}

async fn revalidate(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.revalidate_artifact(&user, &id))
            .await?,
    ))
}

async fn get_claim(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let (claim, dependencies) = state
        .runtime
        .execute_blocking(move |runtime| runtime.get_claim_for(&user, &id))
        .await?;
    Ok(Json(json!({"claim":claim,"dependencies":dependencies})))
}

#[derive(Debug, Deserialize)]
struct ReviewRequest {
    idempotency_key: String,
    claim_ids: Vec<String>,
    decision: ReviewDecision,
    comment: String,
    correction: Option<Value>,
    authority: Authority,
}
#[derive(Debug, Deserialize)]
struct ReviewWithArtifactRequest {
    idempotency_key: String,
    artifact_id: String,
    claim_ids: Vec<String>,
    decision: ReviewDecision,
    comment: String,
    correction: Option<Value>,
    authority: Authority,
}
async fn review(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
    Json(input): Json<ReviewRequest>,
) -> Result<Json<crate::domain::ReviewResult>> {
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
                input.idempotency_key,
            )
            .await?,
    ))
}
async fn review_with_artifact(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(input): Json<ReviewWithArtifactRequest>,
) -> Result<Json<crate::domain::ReviewResult>> {
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
                input.idempotency_key,
            )
            .await?,
    ))
}
async fn list_memory(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
) -> Result<Json<Vec<MemoryObject>>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.memory.list_visible(&user))
            .await?,
    ))
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
    Extension(user): Extension<Identity>,
    Json(input): Json<MemorySearchRequest>,
) -> Result<Json<crate::memory::RetrievalResult>> {
    let end = input.time_end.unwrap_or_else(Utc::now);
    let query = RetrieveQuery {
        task_text: input.task_text,
        required_types: input
            .required_types
            .unwrap_or_default()
            .into_iter()
            .collect(),
        time_start: input.time_start.unwrap_or(end - Duration::days(365)),
        time_end: end,
        max_items: input.max_items.unwrap_or(20).min(100),
    };
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.memory.retrieve(&user, &query))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct VerifySqlRequest {
    request: String,
    sql: String,
}
async fn verify_sql(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(input): Json<VerifySqlRequest>,
) -> Result<Json<crate::domain::SqlPreflight>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| {
                runtime.preflight_sql(&user, &input.request, input.sql)
            })
            .await?,
    ))
}
async fn write_memory(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(object): Json<MemoryObject>,
) -> Result<(StatusCode, Json<MemoryObject>)> {
    let stored = object.clone();
    state
        .runtime
        .execute_blocking(move |runtime| runtime.memory.write(&user, &stored))
        .await?;
    Ok((StatusCode::CREATED, Json(object)))
}
async fn supersede_memory(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
    Json(object): Json<MemoryObject>,
) -> Result<Json<MemoryObject>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.memory.supersede(&user, &id, object))
            .await?,
    ))
}
#[derive(Deserialize)]
struct Limit {
    limit: Option<usize>,
}
async fn audit(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Query(query): Query<Limit>,
) -> Result<Json<Vec<crate::domain::AuditEvent>>> {
    let limit = query.limit.unwrap_or(100);
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| {
                runtime.authorize_operations(&user)?;
                runtime.store.list_audit(&user.tenant_id, limit)
            })
            .await?,
    ))
}
async fn connector_health(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
) -> Result<Json<Value>> {
    state.runtime.authorize_operations(&user)?;
    Ok(Json(json!(state.runtime.connector_health().await?)))
}

async fn metrics(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
) -> Result<Json<crate::observability::MetricsSnapshot>> {
    state.runtime.authorize_operations(&user)?;
    Ok(Json(state.runtime.metrics()))
}

async fn set_retention(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(command): Json<RetentionCommand>,
) -> Result<Json<RetentionRecord>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.set_retention(&user, command))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct ErasureRequest {
    idempotency_key: String,
}

async fn erase_memory(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
    Json(request): Json<ErasureRequest>,
) -> Result<Json<ErasureReceipt>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| {
                runtime.erase_memory(&user, &id, &request.idempotency_key)
            })
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct JobRequest {
    job_type: String,
    payload: Value,
    idempotency_key: String,
}
async fn enqueue_job(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(input): Json<JobRequest>,
) -> Result<(StatusCode, Json<crate::domain::Job>)> {
    let job = state
        .runtime
        .execute_blocking(move |runtime| {
            runtime.authorize_operations(&user)?;
            runtime.scheduler.enqueue(
                &user.tenant_id,
                &input.job_type,
                input.payload,
                input.idempotency_key,
            )
        })
        .await?;
    Ok((StatusCode::CREATED, Json(job)))
}
async fn list_jobs(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Query(query): Query<Limit>,
) -> Result<Json<Vec<crate::domain::Job>>> {
    let limit = query.limit.unwrap_or(100);
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| {
                runtime.authorize_operations(&user)?;
                runtime.store.list_jobs(&user.tenant_id, limit)
            })
            .await?,
    ))
}
#[derive(Debug, Deserialize)]
struct ProcessJobsRequest {
    worker: Option<String>,
    limit: Option<usize>,
}
async fn process_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ProcessJobsRequest>,
) -> Result<Json<Vec<Value>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(state.runtime.process_jobs(
        &user,
        input.worker.as_deref().unwrap_or("ops-worker"),
        input.limit.unwrap_or(10),
    )?))
}
async fn list_outbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Limit>,
) -> Result<Json<Vec<crate::domain::OutboxEvent>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(state.runtime.store.list_pending_outbox(
        &user.tenant_id,
        query.limit.unwrap_or(100),
    )?))
}
#[derive(Debug, Deserialize)]
struct DrainOutboxRequest {
    limit: Option<usize>,
}
async fn drain_outbox(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<DrainOutboxRequest>,
) -> Result<Json<Vec<crate::domain::OutboxEvent>>> {
    let user = identity(&state, &headers)?;
    if !user.roles.contains("admin") {
        return Err(crate::error::AmosError::PermissionDenied(
            "operations role required".into(),
        ));
    }
    Ok(Json(
        state
            .runtime
            .drain_outbox(&user, input.limit.unwrap_or(100))?,
    ))
}
#[derive(Debug, Deserialize)]
struct SourceCursor {
    cursor: Option<String>,
}
async fn process_source_events(
    State(state): State<AppState>,
    Extension(user): Extension<Identity>,
    Json(input): Json<SourceCursor>,
) -> Result<Json<BTreeMap<String, Vec<String>>>> {
    state.runtime.authorize_operations(&user)?;
    Ok(Json(
        state
            .runtime
            .process_source_events(&user, input.cursor.as_deref())
            .await?,
    ))
}
async fn health() -> Json<Value> {
    Json(json!({"status":"ok","runtime":"rust","version":env!("CARGO_PKG_VERSION")}))
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "AMOS API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Authenticated governed-analysis API. Mutations require explicit idempotency keys."
        },
        "servers": [{"url": "/"}],
        "components": {
            "securitySchemes": {
                "bearerAuth": {"type":"http","scheme":"bearer"}
            },
            "schemas": {
                "ErrorEnvelope": {
                    "type":"object",
                    "required":["request_id","error"],
                    "properties":{
                        "request_id":{"type":"string"},
                        "error":{"type":"object","required":["code","message","retryable","review_required"]}
                    }
                },
                "IdempotentCommand": {
                    "type":"object",
                    "required":["idempotency_key"],
                    "properties":{"idempotency_key":{"type":"string","minLength":1,"maxLength":256}}
                }
            }
        },
        "security": [{"bearerAuth":[]}],
        "paths": {
            "/v1/tasks":{"post":{"operationId":"runTask","summary":"Run an idempotent governed task"}},
            "/v1/tasks/{id}":{"get":{"operationId":"getTransaction"}},
            "/v1/artifacts":{"get":{"operationId":"listArtifacts"}},
            "/v1/artifacts/{id}":{"get":{"operationId":"getArtifact"}},
            "/v1/artifacts/{id}/replay":{"post":{"operationId":"replayArtifact"}},
            "/v1/artifacts/{id}/reviews":{"post":{"operationId":"reviewArtifact"}},
            "/v1/memory":{"get":{"operationId":"listMemory"},"post":{"operationId":"writeMemory"}},
            "/v1/memory/search":{"post":{"operationId":"searchMemory"}},
            "/v1/jobs":{"get":{"operationId":"listJobs"},"post":{"operationId":"enqueueJob"}},
            "/v1/retention":{"post":{"operationId":"setRetention"}},
            "/v1/retention/memory/{id}/erase":{"post":{"operationId":"eraseMemory"}},
            "/v1/metrics":{"get":{"operationId":"getMetrics"}}
        }
    }))
}

async fn workspace(Extension(_user): Extension<Identity>) -> Html<String> {
    Html(page(
        "AMOS · Verified analysis",
        r#"<section class="hero"><p class="eyebrow">Payment operations workspace</p><h1>Ask the question.<br><em>Trust the answer.</em></h1><p>AMOS verifies the metric, schema, data state, permissions, and support behind every material claim.</p></section><section class="card"><form method="post" action="/ui/tasks"><label for="request">What do you need to know?</label><textarea id="request" name="request" required>Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?</textarea><button type="submit">Run verified analysis →</button></form></section><section class="grid"><article><b>01</b><h2>Current definitions</h2><p>Approved metrics and active schemas.</p></article><article><b>02</b><h2>Claim evidence</h2><p>Typed support before publication.</p></article><article><b>03</b><h2>Replayable decisions</h2><p>Recorded inputs, versions, and hashes.</p></article></section>"#,
    ))
}
async fn memory_studio(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    let body = state
        .runtime
        .memory
        .list_visible(&identity)?
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
    Ok(Html(page(
        "Memory Studio",
        &format!(
            "<section class='card'><p class='eyebrow'>Memory Studio</p><h1>Governed analytical memory</h1>{body}</section>"
        ),
    )))
}
async fn review_queue(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    state.runtime.authorize_review_queue(&identity)?;
    let artifacts = state.runtime.list_artifacts_for(&identity, 50)?;
    let body=artifacts.into_iter().map(|a|format!("<article><strong>{}</strong><p>{:?}</p><a href='/v1/artifacts/{}'>Inspect evidence</a></article>",escape(&a.title),a.publication_validity,a.artifact_id)).collect::<String>();
    Ok(Html(page(
        "Review Queue",
        &format!(
            "<section class='card'><p class='eyebrow'>Review Queue</p><h1>Human decisions</h1>{body}</section>"
        ),
    )))
}
async fn operations_console(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    state.runtime.authorize_operations(&identity)?;
    let events = state.runtime.store.list_audit(&identity.tenant_id, 50)?;
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
    Ok(Html(page(
        "Operations Console",
        &format!(
            "<section class='card'><p class='eyebrow'>Operations Console</p><h1>Audit and system health</h1>{body}</section>"
        ),
    )))
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
