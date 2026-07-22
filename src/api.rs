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
        .route("/ui/memory/search", post(search_memory_form))
        .route("/ui/memory/notes", post(write_memory_note_form))
        .route("/ui/artifacts/{id}/replay", post(replay_form))
        .route("/ui/artifacts/{id}/reviews", post(review_form))
        .route("/ui/retention", post(set_retention_form))
        .route("/ui/retention/erase", post(erase_memory_form))
        .route(
            "/ui/source-events/process",
            post(process_source_events_form),
        )
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
    idempotency_key: String,
}
async fn run_task_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<TaskForm>,
) -> Result<Html<String>> {
    let result = state
        .runtime
        .run_task(&identity, form.request, form.idempotency_key)
        .await?;
    let replay_key = crate::domain::new_id("ui_replay");
    Ok(Html(page(
        "Verified analysis",
        &format!(
            "<section class='card'><div class='split'><p class='eyebrow'>Lifecycle: {:?}</p><span class='badge warning'>{:?}</span></div><h1>{}</h1><p class='identity'>Signed in as {}</p><pre>{}</pre><dl class='facts'><div><dt>Context</dt><dd>{} tokens · {} objects</dd></div><div><dt>Plan</dt><dd>{} typed steps</dd></div><div><dt>Execution</dt><dd>{} fenced records</dd></div><div><dt>Evidence</dt><dd>{} edges · replay level {}</dd></div></dl><h2>Typed claims</h2>{}<form method='post' action='/ui/artifacts/{}/replay'><input type='hidden' name='idempotency_key' value='{}'><button>Replay into a new A-TXN</button></form></section>",
            result.transaction.state,
            result.transaction.outcome,
            escape(&result.artifact.title),
            escape(&identity.subject_id),
            escape(&result.artifact.content),
            result.manifest.token_count,
            result.manifest.selected_objects.len(),
            result.plan.steps.len(),
            result.executions.len(),
            result.dependencies.len(),
            result.replay_package.replay_level,
            result
                .claims
                .iter()
                .map(|claim| format!(
                    "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>semantic {:?} · publication {:?} · replay {:?}</small></article>",
                    escape(&claim.claim_type),
                    claim.review_state,
                    escape(&claim.text),
                    claim.semantic_validity,
                    claim.publication_validity,
                    claim.replay_availability
                ))
                .collect::<String>(),
            result.artifact.artifact_id,
            replay_key
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct MemorySearchForm {
    task_text: String,
}

async fn search_memory_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<MemorySearchForm>,
) -> Result<Html<String>> {
    let task_text = form.task_text.clone();
    let result = state
        .runtime
        .execute_blocking(move |runtime| {
            let now = Utc::now();
            runtime.memory.retrieve(
                &identity,
                &RetrieveQuery {
                    task_text: form.task_text.clone(),
                    required_types: Default::default(),
                    time_start: now - Duration::days(365),
                    time_end: now,
                    max_items: 20,
                },
            )
        })
        .await?;
    let selected_count = result.items.len();
    let cards = memory_cards(result.items);
    Ok(Html(page(
        "Memory search",
        &format!(
            "<section class='card'><p class='eyebrow'>Permission-first results</p><h1>{}</h1><p>{} governed versions were selected after policy filtering.</p>{}</section>",
            escape(&task_text),
            selected_count,
            empty_state(cards, "No governed memory matched this query.")
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct UiIdempotencyForm {
    idempotency_key: String,
}

async fn replay_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
    Form(command): Form<UiIdempotencyForm>,
) -> Result<Html<String>> {
    let result = state
        .runtime
        .replay_async(&identity, id, command.idempotency_key)
        .await?;
    let comparisons = result
        .comparisons
        .iter()
        .map(|comparison| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>new execution {}</small></article>",
                escape(&comparison.step_id),
                comparison.comparison,
                escape(&comparison.explanation),
                escape(&comparison.replay_execution_id)
            )
        })
        .collect::<String>();
    Ok(Html(page(
        "Replay evidence",
        &format!(
            "<section class='card'><p class='eyebrow'>Replay A-TXN {}</p><h1>Comparison: {:?}</h1><p>The original transaction remains unchanged. Every computation below used a new fence and execution record.</p>{}</section>",
            escape(&result.replay_atxn_id),
            result.status,
            comparisons
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct ReviewForm {
    idempotency_key: String,
    claim_ids: String,
    decision: ReviewDecision,
    comment: String,
    correction: Option<String>,
    confirmation: String,
}

async fn review_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
    Form(form): Form<ReviewForm>,
) -> Result<Html<String>> {
    if form.confirmation != "confirmed" {
        return Err(AmosError::Validation(
            "review confirmation is required".into(),
        ));
    }
    let claim_ids = form
        .claim_ids
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if claim_ids.is_empty() {
        return Err(AmosError::Validation(
            "a review must select at least one claim".into(),
        ));
    }
    let correction = match form.decision {
        ReviewDecision::Correct => {
            let value = form
                .correction
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    AmosError::Validation("a correction requires structured JSON".into())
                })?;
            Some(serde_json::from_str(&value)?)
        }
        ReviewDecision::Approve | ReviewDecision::Reject => None,
    };
    let result = state
        .runtime
        .review_artifact(
            &identity,
            &id,
            claim_ids,
            form.decision,
            form.comment,
            correction,
            Authority::ReviewerApproved,
            form.idempotency_key,
        )
        .await?;
    Ok(Html(page(
        "Review committed",
        &format!(
            "<section class='card'><p class='eyebrow'>Append-only review {}</p><h1>{:?}</h1><p>Lifecycle: {:?}. Publication: {:?}. The original evidence was not mutated.</p><a class='button' href='/reviews'>Return to Review Queue</a></section>",
            escape(&result.review.review_id),
            result.review.decision,
            result.transaction.state,
            result.artifact.publication_validity
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct MemoryNoteForm {
    logical_key: String,
    summary: String,
    content: String,
}

async fn write_memory_note_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<MemoryNoteForm>,
) -> Result<Html<String>> {
    let mut object = MemoryObject::new(
        identity.tenant_id.clone(),
        form.logical_key,
        MemoryType::Document,
        form.summary,
        json!({"text":form.content}),
        format!("user:{}", identity.subject_id),
        crate::domain::new_id("source_version"),
        Authority::UserNote,
    )?;
    object.permissions = identity.permissions.clone();
    object.governing = false;
    let stored = object.clone();
    let write_identity = identity.clone();
    state
        .runtime
        .execute_blocking(move |runtime| runtime.memory.write(&write_identity, &stored))
        .await?;
    Ok(Html(page(
        "Memory note recorded",
        &format!(
            "<section class='card'><p class='eyebrow'>Governed user note</p><h1>{}</h1><p>Version {} was recorded as non-governing, permission-scoped memory with authority {:?}.</p><a class='button' href='/memory'>Return to Memory Studio</a></section>",
            escape(&object.logical_key),
            escape(&object.source_version),
            object.authority
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

async fn process_source_events_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    state.runtime.authorize_operations(&identity)?;
    let result = state.runtime.process_source_events(&identity, None).await?;
    let body = result
        .into_iter()
        .map(|(event_id, claim_ids)| {
            format!(
                "<article><strong>{}</strong><p>{} dependent claims entered bounded revalidation.</p></article>",
                escape(&event_id),
                claim_ids.len()
            )
        })
        .collect::<String>();
    Ok(Html(page(
        "Source events processed",
        &format!(
            "<section class='card'><p class='eyebrow'>Durable connector cursor</p><h1>Source changes processed</h1>{}</section>",
            empty_state(body, "No new source events were available.")
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct RetentionForm {
    idempotency_key: String,
    target_type: String,
    target_id: String,
    retained_until: String,
    legal_hold: Option<String>,
    reason: String,
    confirmation: String,
}

async fn set_retention_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<RetentionForm>,
) -> Result<Html<String>> {
    if form.confirmation != "confirmed" {
        return Err(AmosError::Validation(
            "retention confirmation is required".into(),
        ));
    }
    let retained_until = DateTime::parse_from_rfc3339(&form.retained_until)
        .map_err(|_| AmosError::Validation("retained-until must be RFC 3339".into()))?
        .with_timezone(&Utc);
    let command = RetentionCommand {
        target_type: form.target_type,
        target_id: form.target_id,
        retained_until,
        legal_hold: form.legal_hold.as_deref() == Some("true"),
        reason: form.reason,
        idempotency_key: form.idempotency_key,
    };
    let record = state
        .runtime
        .execute_blocking(move |runtime| runtime.set_retention(&identity, command))
        .await?;
    Ok(Html(page(
        "Retention recorded",
        &format!(
            "<section class='card'><p class='eyebrow'>Atomic privacy control</p><h1>Retention updated</h1><p>{} {} is retained until {}. Legal hold: {}.</p><a class='button' href='/operations'>Return to Operations</a></section>",
            escape(&record.target_type),
            escape(&record.target_id),
            record.retained_until,
            record.legal_hold
        ),
    )))
}

#[derive(Debug, Deserialize)]
struct EraseMemoryForm {
    idempotency_key: String,
    target_id: String,
    confirmation: String,
}

async fn erase_memory_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<EraseMemoryForm>,
) -> Result<Html<String>> {
    if form.confirmation != "confirmed" {
        return Err(AmosError::Validation(
            "erasure confirmation is required".into(),
        ));
    }
    let receipt = state
        .runtime
        .execute_blocking(move |runtime| {
            runtime.erase_memory(&identity, &form.target_id, &form.idempotency_key)
        })
        .await?;
    Ok(Html(page(
        "Erasure complete",
        &format!(
            "<section class='card'><p class='eyebrow'>Erasure receipt {}</p><h1>Memory erased</h1><p>{} dependent claims were atomically invalidated or redacted. Minimum audit proof was retained.</p><a class='button' href='/operations'>Return to Operations</a></section>",
            escape(&receipt.receipt_id),
            receipt.affected_claim_ids.len()
        ),
    )))
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
            "parameters": {
                "ResourceId": {
                    "name":"id","in":"path","required":true,
                    "schema":{"type":"string","minLength":1}
                },
                "Limit": {
                    "name":"limit","in":"query","required":false,
                    "schema":{"type":"integer","minimum":1,"maximum":250}
                }
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
                },
                "TaskCommand": {
                    "allOf":[
                        {"$ref":"#/components/schemas/IdempotentCommand"},
                        {"type":"object","required":["request"],"properties":{"request":{"type":"string","minLength":1}}}
                    ]
                }
            },
            "responses": {
                "Ok": {"description":"Successful governed operation"},
                "Created": {"description":"Durable resource created"},
                "Error": {
                    "description":"Stable AMOS error envelope",
                    "content":{"application/json":{"schema":{"$ref":"#/components/schemas/ErrorEnvelope"}}}
                }
            }
        },
        "security": [{"bearerAuth":[]}],
        "paths": {
            "/v1/openapi.json":{"get":{"operationId":"getOpenApi","summary":"Get this public API contract","security":[],"responses":{"200":{"$ref":"#/components/responses/Ok"}}}},
            "/v1/tasks":{"post":{"operationId":"runTask","summary":"Run an idempotent governed task","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/TaskCommand"}}}},"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/tasks/{id}":{"get":{"operationId":"getTask","summary":"Inspect a task lifecycle","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/transactions/{id}":{"get":{"operationId":"getTransaction","summary":"Inspect an analytical transaction","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts":{"get":{"operationId":"listArtifacts","summary":"List policy-visible artifacts","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts/page":{"get":{"operationId":"listArtifactsPage","summary":"Page through policy-visible artifacts with an opaque cursor","parameters":[{"$ref":"#/components/parameters/Limit"},{"name":"cursor","in":"query","schema":{"type":"string"}}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts/{id}":{"get":{"operationId":"getArtifact","summary":"Inspect artifact claims and dependency evidence","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts/{id}/replay":{"post":{"operationId":"replayArtifact","summary":"Create a separately fenced replay","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/IdempotentCommand"}}}},"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/replay/{id}":{"post":{"operationId":"replayArtifactAlias","summary":"Create a separately fenced replay","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/IdempotentCommand"}}}},"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts/{id}/revalidate":{"post":{"operationId":"revalidateArtifact","summary":"Recompute artifact validity dimensions","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/artifacts/{id}/reviews":{"post":{"operationId":"reviewArtifact","summary":"Commit an idempotent review or correction","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/reviews":{"post":{"operationId":"reviewArtifactWithBodyId","summary":"Commit an idempotent review or correction","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/claims/{id}":{"get":{"operationId":"getClaim","summary":"Inspect a typed claim and dependency evidence","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/memory":{"get":{"operationId":"listMemory","summary":"List policy-visible governed memory","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}},"post":{"operationId":"writeMemory","summary":"Write an authorized immutable memory version","responses":{"201":{"$ref":"#/components/responses/Created"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/memory/search":{"post":{"operationId":"searchMemory","summary":"Permission-first bounded memory search","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/memory/{id}/supersede":{"post":{"operationId":"supersedeMemory","summary":"Append a successor memory version","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/verify/sql":{"post":{"operationId":"verifySql","summary":"Preflight parsed read-only SQL","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/audit":{"get":{"operationId":"listAudit","summary":"List tenant-scoped audit evidence","parameters":[{"$ref":"#/components/parameters/Limit"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/metrics":{"get":{"operationId":"getMetrics","summary":"Get tenant-safe local metrics","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/connectors/health":{"get":{"operationId":"getConnectorHealth","summary":"Inspect connector health and source state","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/jobs":{"get":{"operationId":"listJobs","summary":"List durable jobs","parameters":[{"$ref":"#/components/parameters/Limit"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}},"post":{"operationId":"enqueueJob","summary":"Enqueue an idempotent operator job","responses":{"201":{"$ref":"#/components/responses/Created"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/source-events/process":{"post":{"operationId":"processSourceEvents","summary":"Process a bounded page of durable source events","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/retention":{"post":{"operationId":"setRetention","summary":"Set an idempotent retention or legal-hold record","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/retention/memory/{id}/erase":{"post":{"operationId":"eraseMemory","summary":"Erase due memory and atomically revoke dependents","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/IdempotentCommand"}}}},"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}}
        }
    }))
}

async fn workspace(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    let lookup_identity = identity.clone();
    let artifacts = state
        .runtime
        .execute_blocking(move |runtime| runtime.list_artifacts_for(&lookup_identity, 5))
        .await?;
    let recent = artifacts
        .into_iter()
        .map(|artifact| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>A-TXN {} · {}</small></article>",
                escape(&artifact.title),
                artifact.publication_validity,
                escape(&artifact.artifact_type),
                escape(&artifact.atxn_id),
                artifact.created_at
            )
        })
        .collect::<String>();
    let task_key = crate::domain::new_id("ui_task");
    Ok(Html(page(
        "AMOS · Verified analysis",
        &format!(
            "<section class='hero'><p class='eyebrow'>Payment operations workspace</p><h1>Ask the question.<br><em>Trust the answer.</em></h1><p>AMOS verifies the metric, schema, data state, permissions, and support behind every material claim.</p><p class='identity'>Signed in as <strong>{}</strong> · roles {}</p></section><section class='card'><form method='post' action='/ui/tasks'><input type='hidden' name='idempotency_key' value='{}'><label for='request'>What do you need to know?</label><textarea id='request' name='request' required>Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?</textarea><button type='submit'>Run verified analysis →</button></form></section><section class='grid'><article><b>01</b><h2>Current definitions</h2><p>Approved metrics and active schemas.</p></article><article><b>02</b><h2>Claim evidence</h2><p>Typed support before publication.</p></article><article><b>03</b><h2>Replayable decisions</h2><p>Recorded inputs, versions, and hashes.</p></article></section><section class='card section'><p class='eyebrow'>Recent policy-visible work</p><h2>Analysis history</h2>{}</section>",
            escape(&identity.subject_id),
            escape(
                &identity
                    .roles
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            task_key,
            empty_state(
                recent,
                "No analyses have been admitted for this identity yet."
            )
        ),
    )))
}
async fn memory_studio(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    let lookup_identity = identity.clone();
    let memory = state
        .runtime
        .execute_blocking(move |runtime| runtime.memory.list_visible(&lookup_identity))
        .await?;
    let body = memory_cards(memory);
    Ok(Html(page(
        "Memory Studio",
        &format!(
            "<section class='hero compact'><p class='eyebrow'>Memory Studio</p><h1>Governed analytical memory</h1><p>Only policy-visible versions are shown. Search happens after tenant, status, type, time, and label filtering.</p><p class='identity'>Signed in as <strong>{}</strong></p></section><section class='columns'><section class='card'><h2>Permission-first search</h2><form method='post' action='/ui/memory/search'><label for='task_text'>Search governed memory</label><input id='task_text' name='task_text' required value='payment failure metric'><button type='submit'>Search visible versions</button></form></section><section class='card'><h2>Record a user note</h2><p>Notes are permission-scoped, non-governing memory and cannot override approved definitions.</p><form method='post' action='/ui/memory/notes'><label for='logical_key'>Logical key</label><input id='logical_key' name='logical_key' required value='note:payment-investigation'><label for='summary'>Summary</label><input id='summary' name='summary' required><label for='content'>Note</label><textarea id='content' name='content' required></textarea><button type='submit'>Record governed note</button></form></section></section><section class='card section'><h2>Active versions</h2>{}</section>",
            escape(&identity.subject_id),
            empty_state(body, "No governed memory is visible to this identity.")
        ),
    )))
}
async fn review_queue(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    state.runtime.authorize_review_queue(&identity)?;
    let lookup_identity = identity.clone();
    let artifacts = state
        .runtime
        .execute_blocking(move |runtime| runtime.list_artifacts_for(&lookup_identity, 50))
        .await?;
    let mut body = String::new();
    for artifact in artifacts {
        let artifact_id = artifact.artifact_id.clone();
        let detail_identity = identity.clone();
        let (_, claims, dependencies) = state
            .runtime
            .execute_blocking(move |runtime| {
                runtime.get_artifact_for(&detail_identity, &artifact_id)
            })
            .await?;
        let claim_ids = claims
            .iter()
            .map(|claim| claim.claim_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let claims_body = claims
            .iter()
            .map(|claim| {
                format!(
                    "<li><strong>{}</strong>: {} <span class='badge'>{:?}</span></li>",
                    escape(&claim.claim_type),
                    escape(&claim.text),
                    claim.review_state
                )
            })
            .collect::<String>();
        let review_key = crate::domain::new_id("ui_review");
        body.push_str(&format!(
            "<article><div class='split'><strong>{}</strong><span class='badge warning'>{:?}</span></div><p>{} claims · {} dependency edges · hash {}</p><ul>{}</ul><details><summary>Record a consequential review</summary><form method='post' action='/ui/artifacts/{}/reviews'><input type='hidden' name='idempotency_key' value='{}'><input type='hidden' name='claim_ids' value='{}'><label for='decision-{}'>Decision</label><select id='decision-{}' name='decision'><option value='approve'>Approve and publish</option><option value='reject'>Reject publication</option><option value='correct'>Append correction</option></select><label for='comment-{}'>Reason</label><textarea id='comment-{}' name='comment' required></textarea><label for='correction-{}'>Structured correction (JSON; required for correction)</label><textarea id='correction-{}' name='correction' placeholder='{{&quot;causal_status&quot;:&quot;unproven&quot;}}'></textarea><label class='confirm'><input type='checkbox' name='confirmation' value='confirmed' required> I understand this appends a durable review and may publish, reject, or correct the artifact.</label><button type='submit'>Commit review</button></form></details></article>",
            escape(&artifact.title),
            artifact.publication_validity,
            claims.len(),
            dependencies.len(),
            escape(&artifact.content_hash),
            claims_body,
            escape(&artifact.artifact_id),
            review_key,
            escape(&claim_ids),
            escape(&artifact.artifact_id),
            escape(&artifact.artifact_id),
            escape(&artifact.artifact_id),
            escape(&artifact.artifact_id),
            escape(&artifact.artifact_id),
            escape(&artifact.artifact_id)
        ));
    }
    Ok(Html(page(
        "Review Queue",
        &format!(
            "<section class='hero compact'><p class='eyebrow'>Review Queue</p><h1>Human decisions</h1><p>Inspect typed support before making an append-only decision. Original evidence is immutable.</p><p class='identity'>Signed in as <strong>{}</strong></p></section><section class='card'>{}</section>",
            escape(&identity.subject_id),
            empty_state(body, "No policy-visible artifacts await inspection.")
        ),
    )))
}
async fn operations_console(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    state.runtime.authorize_operations(&identity)?;
    let tenant_id = identity.tenant_id.clone();
    let (events, jobs, outbox) = state
        .runtime
        .execute_blocking(move |runtime| {
            Ok((
                runtime.store.list_audit(&tenant_id, 50)?,
                runtime.store.list_jobs(&tenant_id, 50)?,
                runtime.store.list_outbox(&tenant_id, 50)?,
            ))
        })
        .await?;
    let health = state.runtime.connector_health().await?;
    let metrics = state.runtime.metrics();
    let audit_body = events
        .into_iter()
        .map(|e| {
            format!(
                "<article><strong>{}</strong><p>{} · {} · {}</p></article>",
                escape(&e.action),
                escape(&e.actor_id),
                escape(&e.outcome),
                e.created_at
            )
        })
        .collect::<String>();
    let jobs_body = jobs
        .into_iter()
        .map(|job| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>attempt {} / {} · fence {}</p></article>",
                escape(&job.job_type), job.state, job.attempt, job.max_attempts, job.fencing_token
            )
        })
        .collect::<String>();
    let outbox_body = outbox
        .into_iter()
        .map(|event| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{} · attempt {} / {}</p></article>",
                escape(&event.event_type), event.state, escape(&event.aggregate_id), event.attempt, event.max_attempts
            )
        })
        .collect::<String>();
    let health_body = format!(
        "<article><div class='split'><strong>{}</strong><span class='badge'>{}</span></div><p>lag {}s · degraded capabilities {}</p></article>",
        escape(&health.source_id),
        escape(&health.status),
        health.lag_seconds,
        health.degraded_capabilities.len()
    );
    let retention_key = crate::domain::new_id("ui_retention");
    let erasure_key = crate::domain::new_id("ui_erasure");
    Ok(Html(page(
        "Operations Console",
        &format!(
            "<section class='hero compact'><p class='eyebrow'>Operations Console</p><h1>Durable control plane</h1><p>Signed in as <strong>{}</strong>. Inspect connector health, lifecycle counters, jobs, delivery, and append-only audit evidence.</p></section><section class='grid metrics'><article><b>{}</b><h2>Tasks passed</h2></article><article><b>{}</b><h2>Tasks failed</h2></article><article><b>{}</b><h2>Recoveries</h2></article></section><section class='card section'><div class='split'><h2>Connector health</h2><form method='post' action='/ui/source-events/process'><button type='submit'>Process source changes</button></form></div>{}</section><section class='columns'><section class='card'><h2>Jobs</h2>{}</section><section class='card'><h2>Outbox delivery</h2>{}</section></section><section class='card section'><h2>Retention and privacy</h2><div class='columns'><form method='post' action='/ui/retention'><h3>Set retention or legal hold</h3><input type='hidden' name='idempotency_key' value='{}'><input type='hidden' name='target_type' value='memory'><label for='retention-target'>Memory object ID</label><input id='retention-target' name='target_id' required><label for='retained-until'>Retained until (RFC 3339)</label><input id='retained-until' name='retained_until' required value='2030-01-01T00:00:00Z'><label for='retention-reason'>Reason</label><textarea id='retention-reason' name='reason' required></textarea><label class='confirm'><input type='checkbox' name='legal_hold' value='true'> Apply legal hold</label><label class='confirm'><input type='checkbox' name='confirmation' value='confirmed' required> I confirm this tenant-scoped retention change.</label><button type='submit'>Commit retention</button></form><form method='post' action='/ui/retention/erase'><h3>Erase due memory</h3><input type='hidden' name='idempotency_key' value='{}'><p>Erasure fails closed while retained or under legal hold.</p><label for='erasure-target'>Memory object ID</label><input id='erasure-target' name='target_id' required><label class='confirm'><input type='checkbox' name='confirmation' value='confirmed' required> I confirm this irreversible content erasure.</label><button type='submit'>Erase due memory</button></form></div></section><section class='card section'><h2>Audit trail</h2>{}</section>",
            escape(&identity.subject_id),
            metrics.task_succeeded,
            metrics.task_failed,
            metrics.recovery_succeeded,
            empty_state(health_body, "No configured connectors reported health."),
            empty_state(jobs_body, "No durable jobs are queued."),
            empty_state(outbox_body, "No outbox events have been committed."),
            retention_key,
            erasure_key,
            empty_state(audit_body, "No audit events have been recorded.")
        ),
    )))
}
fn memory_cards(memory: Vec<MemoryObject>) -> String {
    memory
        .into_iter()
        .map(|object| {
            let effective = match (object.effective_start, object.effective_end) {
                (Some(start), Some(end)) => format!("{start} → {end}"),
                (Some(start), None) => format!("from {start}"),
                (None, Some(end)) => format!("until {end}"),
                (None, None) => "not time-bounded".into(),
            };
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><dl class='facts'><div><dt>Version</dt><dd>{} · {}</dd></div><div><dt>Authority</dt><dd>{:?}</dd></div><div><dt>Effective</dt><dd>{}</dd></div><div><dt>Provenance</dt><dd>{}</dd></div></dl></article>",
                escape(&object.logical_key),
                object.status,
                escape(&object.summary),
                escape(&object.version),
                escape(&object.source_version),
                object.authority,
                escape(&effective),
                escape(object.provenance_ref.as_deref().unwrap_or("direct source observation"))
            )
        })
        .collect()
}
fn empty_state(body: String, message: &str) -> String {
    if body.is_empty() {
        format!("<p class='empty'>{}</p>", escape(message))
    } else {
        body
    }
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
const STYLE: &str = r#":root{font-family:Inter,ui-sans-serif,system-ui;color:#17231c;background:#f6f4ed}*{box-sizing:border-box}body{margin:0}header{min-height:68px;padding:14px 5vw;display:flex;align-items:center;justify-content:space-between;gap:20px;border-bottom:1px solid #d9d9cf}header>a{font:700 22px Georgia;color:#17231c;text-decoration:none}nav{display:flex;flex-wrap:wrap;gap:18px}nav a{font-size:13px;color:#526159;text-decoration:none}a{color:#1f5a3d}a:focus-visible,button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,summary:focus-visible{outline:3px solid #c58b39;outline-offset:3px}main{width:min(1040px,90vw);margin:60px auto}.hero{max-width:760px}.hero.compact h1{font-size:clamp(40px,6vw,64px)}.eyebrow{text-transform:uppercase;letter-spacing:.13em;color:#1f5a3d;font-size:11px;font-weight:800}h1{font:400 clamp(44px,7vw,76px)/1 Georgia;margin:18px 0}h1 em{color:#1f5a3d}h2{font:400 26px Georgia}.hero>p{color:#66736d;line-height:1.7;max-width:680px}.identity{padding:10px 14px;border-left:3px solid #1f5a3d;background:#eaf0e9}.card{min-width:0;background:#fffefa;border:1px solid #d9d9cf;border-radius:18px;padding:32px;box-shadow:0 18px 50px #1c2c2214}.card h1{font-size:38px}.section{margin-top:28px}.columns{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:24px;margin-top:24px}.columns>*{min-width:0}.split{display:flex;align-items:center;justify-content:space-between;gap:18px}.split form,.split button{margin:0}label{display:block;font-size:12px;font-weight:700;margin:12px 0 7px}textarea,select,input{width:100%;padding:14px;border:1px solid #bfc5bc;border-radius:10px;background:#faf9f4;color:#17231c}textarea{min-height:120px;font:20px Georgia}.confirm{display:flex;align-items:flex-start;gap:9px;font-weight:500;line-height:1.5}.confirm input{width:auto;margin-top:3px}button,.button{display:inline-block;margin-top:16px;padding:13px 18px;border:0;border-radius:10px;background:#17231c;color:#fff;font-weight:700;text-decoration:none;cursor:pointer}.grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));margin-top:35px;border-top:1px solid #d9d9cf}.grid article{padding:26px}.grid article+article{border-left:1px solid #d9d9cf}.metrics b{font:400 42px Georgia;color:#1f5a3d}article{min-width:0;padding:14px 0;border-bottom:1px solid #e7e6de;overflow-wrap:anywhere}article p,small,.empty{color:#66736d}.badge{display:inline-block;padding:5px 9px;border-radius:999px;background:#e8efe8;color:#1f5a3d;font-size:11px;font-weight:800}.badge.warning{background:#f5e8ca;color:#7d5617}.facts{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin:18px 0}.facts div{padding:12px;background:#f7f5ee;border-radius:8px}.facts dt{font-size:11px;text-transform:uppercase;letter-spacing:.08em;color:#66736d}.facts dd{margin:5px 0 0;overflow-wrap:anywhere}details{margin-top:15px;padding:14px;border:1px solid #d9d9cf;border-radius:10px}summary{cursor:pointer;font-weight:700}pre{white-space:pre-wrap;line-height:1.6;overflow-wrap:anywhere}.error{border-color:#a86666}@media(max-width:700px){header{align-items:flex-start;flex-direction:column}nav{gap:12px}main{margin:35px auto}.grid,.columns,.facts{grid-template-columns:minmax(0,1fr)}.grid article+article{border-left:0;border-top:1px solid #d9d9cf}.card{padding:22px}.split{align-items:flex-start;flex-direction:column}}"#;
