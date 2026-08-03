use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use axum::{
    Form, Json, Router,
    extract::{DefaultBodyLimit, Extension, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, COOKIE, LOCATION, SET_COOKIE,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Result,
    artifacts::VerifiedFactCatalog,
    auth::{IdentityProvider, StaticIdentityProvider},
    domain::{
        AnalyticalTransaction, Artifact, AuditEvent, Authority, Claim, ContextManifest,
        DependencyEdge, ErasureReceipt, ExecutionRecord, Identity, Job, MemoryObject, MemoryType,
        OutboxEvent, ReplayPackage, RetentionCommand, RetentionRecord, ReviewDecision, TypedPlan,
        VerificationRecord,
    },
    error::AmosError,
    memory::RetrieveQuery,
    model::ModelInvocation,
    packs::AnalysisPack,
    runtime::AmosRuntime,
};

pub use crate::auth::demo_identities;

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<AmosRuntime>,
    identity_provider: Arc<dyn IdentityProvider>,
    demo_sessions: Option<DemoSessions>,
}

pub fn router(runtime: AmosRuntime, identity_provider: Arc<dyn IdentityProvider>) -> Router {
    build_router(runtime, identity_provider, None)
}

#[derive(Clone, Default)]
struct DemoSessions {
    identities_by_session: Arc<Mutex<BTreeMap<String, String>>>,
}

fn build_router(
    runtime: AmosRuntime,
    identity_provider: Arc<dyn IdentityProvider>,
    demo_sessions: Option<DemoSessions>,
) -> Router {
    let state = AppState {
        runtime: Arc::new(runtime),
        identity_provider,
        demo_sessions: demo_sessions.clone(),
    };
    let protected = Router::new()
        .route("/", get(workspace))
        .route("/analyses/{id}", get(analysis_detail))
        .route("/claims/{id}", get(claim_evidence))
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
        .route("/v1/packs", get(list_packs).post(install_pack))
        .route("/v1/packs/{id}", get(get_pack));
    let protected = if demo_sessions.is_some() {
        protected
            .route("/demo/source-change", post(trigger_demo_source_change_form))
            .route("/advisor-demo", get(advisor_demo_workspace))
            .route("/advisor-demo/analyze", post(run_advisor_demo_form))
    } else {
        protected
    }
    .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let public = Router::new()
        .route("/health", get(health))
        .route("/v1/openapi.json", get(openapi));
    let public = if demo_sessions.is_some() {
        public
            .route("/demo/login", get(demo_login))
            .route("/demo/identity", post(switch_demo_identity))
    } else {
        public
    };
    public
        .merge(protected)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn(request_controls))
}

pub fn demo_router(runtime: AmosRuntime) -> Router {
    build_router(
        runtime,
        Arc::new(StaticIdentityProvider::demo()),
        Some(DemoSessions::default()),
    )
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
    match authenticate_request(&state, request.headers()) {
        Ok(identity) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

fn authenticate_request(state: &AppState, headers: &HeaderMap) -> Result<Identity> {
    if headers.contains_key(AUTHORIZATION) {
        return bearer_token(headers)
            .and_then(|token| state.identity_provider.authenticate_bearer(token));
    }
    if let Some(sessions) = &state.demo_sessions
        && let Some(session_id) = demo_session_cookie(headers)?
    {
        let identity_key = sessions
            .identities_by_session
            .lock()
            .map_err(|_| AmosError::Storage("demo session registry is unavailable".into()))?
            .get(session_id)
            .cloned()
            .ok_or_else(|| AmosError::Unauthenticated("demo session is invalid".into()))?;
        return state.identity_provider.authenticate_bearer(&identity_key);
    }
    Err(AmosError::Unauthenticated(
        "bearer credentials are required".into(),
    ))
}

fn demo_session_cookie(headers: &HeaderMap) -> Result<Option<&str>> {
    let Some(raw_cookie) = headers.get(COOKIE) else {
        return Ok(None);
    };
    let raw_cookie = raw_cookie
        .to_str()
        .map_err(|_| AmosError::Unauthenticated("cookie header is invalid".into()))?;
    Ok(raw_cookie.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == "amos_demo_session"
            && !value.is_empty()
            && value.len() <= 160
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            }))
        .then_some(value)
    }))
}

#[derive(Debug, Deserialize)]
struct DemoIdentityForm {
    identity: String,
}

async fn demo_login(State(state): State<AppState>) -> Result<Html<String>> {
    if state.demo_sessions.is_none() {
        return Err(AmosError::NotFound("demo login".into()));
    }
    Ok(Html(page(
        "AMOS · Local demo",
        r#"
<section class="hero compact">
  <p class="eyebrow">Local concept environment</p>
  <h1>Choose a demo role.</h1>
  <p>This creates an opaque, server-side local session. The advisor experience uses only synthetic client and cohort data.</p>
</section>
<section class="card">
  <h2>Enter the banking advisor walkthrough</h2>
  <p>Start as the relationship advisor to ask AMOS for a client briefing.</p>
  <form method="post" action="/demo/identity">
    <input type="hidden" name="identity" value="analyst_001">
    <button type="submit">Continue as Advisor →</button>
  </form>
</section>
"#,
    )))
}

async fn switch_demo_identity(
    State(state): State<AppState>,
    Form(form): Form<DemoIdentityForm>,
) -> Result<Response> {
    if !matches!(
        form.identity.as_str(),
        "analyst_001" | "reviewer_001" | "admin"
    ) {
        return Err(AmosError::Validation(
            "unsupported local demo identity".into(),
        ));
    }
    let sessions = state
        .demo_sessions
        .as_ref()
        .ok_or_else(|| AmosError::NotFound("demo identity switch".into()))?;
    state
        .identity_provider
        .authenticate_bearer(&form.identity)?;
    let session_id = crate::domain::new_id("demo_session");
    let mut registry = sessions
        .identities_by_session
        .lock()
        .map_err(|_| AmosError::Storage("demo session registry is unavailable".into()))?;
    if registry.len() >= 128 {
        registry.clear();
    }
    registry.insert(session_id.clone(), form.identity);
    drop(registry);
    let cookie = format!("amos_demo_session={session_id}; Path=/; HttpOnly; SameSite=Strict");
    let mut response = StatusCode::SEE_OTHER.into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|_| AmosError::Storage("demo session cookie is invalid".into()))?,
    );
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static("/"));
    Ok(response)
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
    #[serde(default)]
    task_type: Option<String>,
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
            .run_task_typed(
                &identity,
                request.request,
                request.task_type,
                request.idempotency_key,
            )
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
) -> Result<Redirect> {
    let result = state
        .runtime
        .run_task(&identity, form.request, form.idempotency_key)
        .await?;
    Ok(Redirect::to(&format!(
        "/analyses/{}",
        result.artifact.artifact_id
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
            "<section class='card'><p class='eyebrow'>Append-only review {}</p><h1>{:?}</h1><div class='status-line'><span class='badge'>Lifecycle {:?}</span><span class='badge'>Publication {:?}</span></div><p>The original evidence was not mutated. The review, reviewer identity, authority, selected claims, reason, and resulting publication state are durable.</p><a class='button' href='/analyses/{}'>Open published analysis</a> <a class='button' href='/reviews'>Return to Review Queue</a></section>",
            escape(&result.review.review_id),
            result.review.decision,
            result.transaction.state,
            result.artifact.publication_validity,
            escape(&result.artifact.artifact_id)
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
) -> Result<Json<crate::runtime::ClaimEvidenceView>> {
    Ok(Json(
        state
            .runtime
            .execute_blocking(move |runtime| runtime.claim_evidence_view(&user, &id))
            .await?,
    ))
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
struct DemoSourceChangeForm {
    artifact_id: String,
    idempotency_key: String,
}

async fn trigger_demo_source_change_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<DemoSourceChangeForm>,
) -> Result<Redirect> {
    let artifact_id = form.artifact_id;
    let lookup_identity = identity.clone();
    let checked_artifact_id = artifact_id.clone();
    state
        .runtime
        .execute_blocking(move |runtime| {
            runtime.get_artifact_for(&lookup_identity, &checked_artifact_id)?;
            let result =
                runtime.trigger_demo_source_change(&lookup_identity, &form.idempotency_key)?;
            if !result.affected_artifact_ids.contains(&checked_artifact_id) {
                return Err(AmosError::Conflict(
                    "the governed source successor did not affect the selected artifact".into(),
                ));
            }
            Ok(result)
        })
        .await?;
    Ok(Redirect::to(&format!("/analyses/{artifact_id}")))
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
async fn health(State(state): State<AppState>) -> Result<Json<Value>> {
    let descriptor = state.runtime.model_descriptor();
    let boundary = state.runtime.privacy_boundary()?;
    let runtime = state.runtime.clone();
    let (schema_version, model_compatibility_probe_passed) = state
        .runtime
        .execute_blocking(move |_| {
            Ok((
                runtime.store.schema_version()?,
                runtime.model_compatibility_probe_passed()?,
            ))
        })
        .await?;
    let warehouse = state.runtime.connector_health().await?;
    Ok(Json(json!({
        "status": "ok",
        "runtime": "rust",
        "app_version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema_version,
        "model": {
            "provider": descriptor.provider,
            "name": descriptor.model,
            "route_class": descriptor.route_class,
            "compatibility_probe_passed": model_compatibility_probe_passed,
        },
        "warehouse": warehouse,
        "external_telemetry": boundary.external_telemetry,
        "allowed_egress_hosts": boundary.public_egress_allowlist,
    })))
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
            "/v1/retention/memory/{id}/erase":{"post":{"operationId":"eraseMemory","summary":"Erase due memory and atomically revoke dependents","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/IdempotentCommand"}}}},"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/packs":{"get":{"operationId":"listPacks","summary":"List installed analysis packs","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}},"post":{"operationId":"installPack","summary":"Install a schema-validated analysis pack","responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}},
            "/v1/packs/{id}":{"get":{"operationId":"getPack","summary":"Inspect an installed analysis pack","parameters":[{"$ref":"#/components/parameters/ResourceId"}],"responses":{"200":{"$ref":"#/components/responses/Ok"},"4XX":{"$ref":"#/components/responses/Error"}}}}
        }
    }))
}

#[derive(Debug, Deserialize)]
struct InstallPackRequest {
    pack: Value,
}

async fn list_packs(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Value>> {
    state.runtime.authorize_operations(&identity)?;
    let packs = state.runtime.list_installed_packs(&identity)?;
    Ok(Json(json!({ "items": packs })))
}

async fn get_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(pack_id): Path<String>,
) -> Result<Json<AnalysisPack>> {
    state.runtime.authorize_operations(&identity)?;
    Ok(Json(state.runtime.get_installed_pack(&identity, &pack_id)?))
}

async fn install_pack(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(request): Json<InstallPackRequest>,
) -> Result<Json<Value>> {
    let bytes = serde_json::to_vec(&request.pack)?;
    let pack = AnalysisPack::from_json_slice(&bytes)?;
    let (newly_installed, pack) = state.runtime.install_pack(&identity, pack)?;
    Ok(Json(json!({
        "newly_installed": newly_installed,
        "pack_id": pack.pack_id,
        "task_type": pack.task_type,
        "version": pack.version,
    })))
}

struct AnalysisViewData {
    transaction: AnalyticalTransaction,
    artifact: Artifact,
    claims: Vec<Claim>,
    dependencies: Vec<DependencyEdge>,
    manifest: ContextManifest,
    plan: TypedPlan,
    executions: Vec<ExecutionRecord>,
    verifications: Vec<VerificationRecord>,
    invocations: Vec<ModelInvocation>,
    facts: Option<VerifiedFactCatalog>,
    replay: ReplayPackage,
    audit: Vec<AuditEvent>,
    jobs: Vec<Job>,
    outbox: Vec<OutboxEvent>,
}

fn load_analysis(
    runtime: &AmosRuntime,
    identity: &Identity,
    artifact_id: &str,
) -> Result<AnalysisViewData> {
    let (artifact, claims, dependencies) = runtime.get_artifact_for(identity, artifact_id)?;
    let transaction = runtime.get_transaction_for(identity, &artifact.atxn_id)?;
    let manifest = runtime
        .store
        .get_manifest_by_atxn(&identity.tenant_id, &artifact.atxn_id)?
        .ok_or_else(|| AmosError::NotFound(format!("manifest for {}", artifact.atxn_id)))?;
    let plan = runtime
        .store
        .get_plan_by_atxn(&identity.tenant_id, &artifact.atxn_id)?
        .ok_or_else(|| AmosError::NotFound(format!("plan for {}", artifact.atxn_id)))?;
    let executions = runtime
        .store
        .list_executions(&identity.tenant_id, &artifact.atxn_id)?;
    let verifications = runtime
        .store
        .list_verifications(&identity.tenant_id, &artifact.atxn_id)?;
    let invocations = runtime
        .store
        .list_model_invocations(&identity.tenant_id, &artifact.atxn_id)?;
    let facts = runtime
        .store
        .get_verified_fact_catalog(&identity.tenant_id, &artifact.atxn_id)?;
    let replay = runtime
        .store
        .get_replay_package(&identity.tenant_id, artifact_id)?
        .ok_or_else(|| AmosError::NotFound(format!("replay package for {artifact_id}")))?;
    let claim_ids = claims
        .iter()
        .map(|claim| claim.claim_id.clone())
        .collect::<Vec<_>>();
    let audit = runtime
        .store
        .list_audit(&identity.tenant_id, 200)?
        .into_iter()
        .filter(|event| {
            event.atxn_id.as_deref() == Some(artifact.atxn_id.as_str())
                || event.target_id == artifact.artifact_id
                || claims.iter().any(|claim| claim.claim_id == event.target_id)
                || event
                    .details
                    .get("affected_artifact_ids")
                    .and_then(Value::as_array)
                    .is_some_and(|ids| {
                        ids.iter()
                            .any(|id| id.as_str() == Some(artifact.artifact_id.as_str()))
                    })
        })
        .collect();
    let jobs = runtime
        .store
        .list_jobs(&identity.tenant_id, 250)?
        .into_iter()
        .filter(|job| {
            job.payload.get("artifact_id").and_then(Value::as_str)
                == Some(artifact.artifact_id.as_str())
                || job
                    .payload
                    .get("claim_id")
                    .and_then(Value::as_str)
                    .is_some_and(|claim_id| claim_ids.iter().any(|id| id == claim_id))
        })
        .collect::<Vec<_>>();
    let invalidation_keys = jobs
        .iter()
        .filter_map(|job| {
            job.payload
                .get("invalidation_key")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    let outbox = runtime
        .store
        .list_outbox(&identity.tenant_id, 500)?
        .into_iter()
        .filter(|event| {
            event.aggregate_id == artifact.artifact_id
                || claim_ids.contains(&event.aggregate_id)
                || event.payload.get("artifact_id").and_then(Value::as_str)
                    == Some(artifact.artifact_id.as_str())
                || invalidation_keys
                    .iter()
                    .any(|key| event.idempotency_key.contains(key))
        })
        .collect();
    Ok(AnalysisViewData {
        transaction,
        artifact,
        claims,
        dependencies,
        manifest,
        plan,
        executions,
        verifications,
        invocations,
        facts,
        replay,
        audit,
        jobs,
        outbox,
    })
}

async fn analysis_detail(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Html<String>> {
    let lookup_identity = identity.clone();
    let analysis = state
        .runtime
        .execute_blocking(move |runtime| load_analysis(runtime, &lookup_identity, &id))
        .await?;
    let boundary = state.runtime.privacy_boundary()?;
    let allowed_egress = if boundary.public_egress_allowlist.is_empty() {
        "none".into()
    } else {
        boundary.public_egress_allowlist.join(", ")
    };
    let blocked_fields = state.runtime.blocked_analysis_fields().join(", ");
    let selected_context = analysis
        .manifest
        .selected_objects
        .iter()
        .map(|object| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>{} · source version {} · object {}</small></article>",
                escape(&object.logical_key),
                object.memory_type,
                escape(&object.summary),
                escape(&object.source_id),
                escape(&object.source_version),
                escape(&object.object_id)
            )
        })
        .collect::<String>();
    let model_invocations = analysis
        .invocations
        .iter()
        .map(|invocation| {
            let payload_contract = match invocation.purpose {
                crate::model::ModelPurpose::Plan => format!(
                    "{} governed objects · 0 raw warehouse rows",
                    invocation.selected_object_ids.len()
                ),
                crate::model::ModelPurpose::Narrative => format!(
                    "{} verified aggregate results · 0 raw warehouse rows",
                    invocation.verified_execution_ids.len()
                ),
            };
            Ok(format!(
                "<article><div class='split'><strong>{:?} · attempt {}</strong><span class='badge'>{:?}</span></div><p class='boundary-note'>{}</p><dl class='facts'><div><dt>Invocation ID</dt><dd>{}</dd></div><div><dt>Live model</dt><dd>{}:{}</dd></div><div><dt>Route</dt><dd>{:?}</dd></div><div><dt>Latency</dt><dd>{} ms</dd></div><div><dt>Tokens</dt><dd>{} input · {} output</dd></div><div><dt>Manifest / prompt hash</dt><dd>{}</dd></div><div><dt>Input payload hash</dt><dd>{}</dd></div><div><dt>Response hash</dt><dd>{}</dd></div></dl><details><summary>Permission-filtered model payload</summary><pre>{}</pre></details></article>",
                invocation.purpose,
                invocation.attempt,
                invocation.status,
                escape(&payload_contract),
                escape(&invocation.invocation_id),
                escape(&invocation.provider),
                escape(&invocation.model),
                invocation.route_class,
                invocation.latency_ms,
                invocation.input_tokens,
                invocation.output_tokens,
                escape(&invocation.input_manifest_hash),
                escape(&invocation.input_payload_hash),
                escape(invocation.output_hash.as_deref().unwrap_or("not recorded")),
                pretty_json(&invocation.sanitized_input)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("");
    let proposed_steps = analysis
        .plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let relations = step
                .parameters
                .get("relations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(escape)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "<article><div class='split'><strong>{:02} · {}</strong><span class='badge'>Proposed by Gemma 4</span></div><p>{}</p><dl class='facts'><div><dt>Expected columns</dt><dd>{}</dd></div><div><dt>Requested relation</dt><dd>{}</dd></div></dl></article>",
                index + 1,
                escape(&step.step_id),
                escape(&step.purpose),
                escape(&step.expected_output_schema),
                relations
            )
        })
        .collect::<String>();
    let plan_steps = analysis
        .plan
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let relations = step
                .parameters
                .get("relations")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(escape)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "admitted relation encoded in SQL".into());
            let sql = step
                .parameters
                .get("sql")
                .and_then(Value::as_str)
                .unwrap_or("No SQL parameter");
            let execution = analysis
                .executions
                .iter()
                .find(|execution| execution.step_id == step.step_id);
            let verification = execution.and_then(|record| {
                analysis.verifications.iter().find(|verification| {
                    verification.execution_id.as_deref() == Some(record.execution_id.as_str())
                })
            });
            let execution_body = execution
                .map(|record| {
                    format!(
                        "<dl class='facts'><div><dt>Execution</dt><dd>{}</dd></div><div><dt>Fencing token</dt><dd>{}</dd></div><div><dt>Rows / bytes</dt><dd>{} / {}</dd></div><div><dt>Output hash</dt><dd>{}</dd></div></dl><details><summary>Verified aggregate result</summary><pre>{}</pre></details>",
                        escape(&record.execution_id),
                        record.fencing_token,
                        record.row_count,
                        record.byte_count,
                        escape(&record.output_hash),
                        pretty_json(&record.output).unwrap_or_else(|error| escape(&error.to_string()))
                    )
                })
                .unwrap_or_else(|| "<p class='empty'>No execution record.</p>".into());
            let checks = verification
                .map(|record| {
                    record
                        .checks
                        .iter()
                        .map(|check| {
                            format!(
                                "<li><span class='badge'>{:?}</span> <strong>{}</strong>{}</li>",
                                check.outcome,
                                escape(&check.rule_id),
                                check
                                    .message
                                    .as_ref()
                                    .map(|message| format!(" · {}", escape(message)))
                                    .unwrap_or_default()
                            )
                        })
                        .collect::<String>()
                })
                .unwrap_or_else(|| "<li>No verification record.</li>".into());
            format!(
                "<article class='step'><p class='eyebrow'>Step {:02} · AMOS admitted</p><h3>{}</h3><p>{}</p><dl class='facts'><div><dt>Tool</dt><dd>{}</dd></div><div><dt>Source</dt><dd>{}</dd></div><div><dt>Capability relation</dt><dd>{}</dd></div><div><dt>Capability limits</dt><dd>{}s · {} rows · {} bytes</dd></div></dl><details open><summary>Exact read-only SQL</summary><pre>{}</pre></details><p class='boundary-note'>Narrow capability issued after policy admission. Signature and warehouse credential are never rendered.</p>{}<h4>AMOS verification checks</h4><ul class='checks'>{}</ul>",
                index + 1,
                escape(&step.purpose),
                escape(&step.verifier_profile),
                escape(&step.tool),
                escape(&step.source_id),
                relations,
                step.limits.seconds,
                step.limits.rows,
                step.limits.bytes,
                escape(sql),
                execution_body,
                checks
            )
        })
        .collect::<String>();
    let claims = analysis
        .claims
        .iter()
        .map(|claim| {
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>{:?} semantic · {:?} publication · {} execution supports</small><br><a class='text-link' href='/claims/{}'>Open complete claim evidence →</a></article>",
                escape(&claim.claim_type),
                claim.review_state,
                escape(&claim.text),
                claim.semantic_validity,
                claim.publication_validity,
                claim.support_execution_ids.len(),
                escape(&claim.claim_id)
            )
        })
        .collect::<String>();
    let facts = analysis
        .facts
        .as_ref()
        .map(|catalog| {
            catalog
                .facts
                .iter()
                .map(|fact| {
                    format!(
                        "<article><strong>{}</strong><p>{}</p><small>{} · {} sources</small></article>",
                        escape(&fact.fact_id),
                        escape(&fact.canonical_text),
                        escape(&fact.claim_type),
                        fact.source_versions.len()
                    )
                })
                .collect::<String>()
        })
        .unwrap_or_else(|| "<p class='empty'>No verified fact catalog was required.</p>".into());
    let omissions = analysis
        .manifest
        .omissions
        .iter()
        .map(|omission| {
            format!(
                "<li><strong>{}</strong> · {}</li>",
                escape(&omission.role),
                escape(&omission.reason)
            )
        })
        .collect::<String>();
    let audit = analysis
        .audit
        .iter()
        .map(|event| {
            format!(
                "<li><strong>{}</strong> · {} · {} · {}</li>",
                escape(&event.action),
                escape(&event.actor_id),
                escape(&event.outcome),
                event.created_at
            )
        })
        .collect::<String>();
    let artifact_body = if analysis.artifact.artifact_type == "html_report" {
        analysis.artifact.content.clone()
    } else {
        format!("<pre>{}</pre>", escape(&analysis.artifact.content))
    };
    let stale_claims = analysis
        .claims
        .iter()
        .filter(|claim| {
            matches!(
                claim.semantic_validity,
                crate::domain::SemanticValidity::PendingRevalidation
                    | crate::domain::SemanticValidity::Stale
            )
        })
        .collect::<Vec<_>>();
    let source_change_panel = if stale_claims.is_empty()
        && state.demo_sessions.is_some()
        && identity.roles.contains("admin")
    {
        format!(
            "<section class='card section source-change'><p class='eyebrow'>Governed demo source</p><h2>Receive successor subscription snapshot</h2><p>This append-only action supersedes the selected governed snapshot, traverses reverse claim dependencies, enqueues bounded revalidation, and preserves the published artifact and replay package.</p><form method='post' action='/demo/source-change'><input type='hidden' name='artifact_id' value='{}'><input type='hidden' name='idempotency_key' value='{}'><label class='confirm'><input type='checkbox' required> I understand dependent claims will become stale without rewriting their evidence.</label><button type='submit'>Receive updated snapshot</button></form></section>",
            escape(&analysis.artifact.artifact_id),
            crate::domain::new_id("demo_source_change")
        )
    } else if stale_claims.is_empty() {
        String::new()
    } else {
        let stale = stale_claims
            .iter()
            .map(|claim| {
                format!(
                    "<li><a href='/claims/{}'>{}</a> <span class='badge warning'>{:?}</span></li>",
                    escape(&claim.claim_id),
                    escape(&claim.claim_type),
                    claim.semantic_validity
                )
            })
            .collect::<String>();
        let jobs = analysis
            .jobs
            .iter()
            .filter(|job| {
                matches!(
                    job.job_type.as_str(),
                    "claim.revalidate" | "invalidation.continue"
                )
            })
            .map(|job| {
                format!(
                    "<li><strong>{}</strong> · {:?} · attempt {} · fence {} · {}</li>",
                    escape(&job.job_type),
                    job.state,
                    job.attempt,
                    job.fencing_token,
                    escape(&job.job_id)
                )
            })
            .collect::<String>();
        let outbox = analysis
            .outbox
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "claim.validity_changed" | "invalidation.processed"
                )
            })
            .map(|event| {
                format!(
                    "<li><strong>{}</strong> · {:?} · {} · {}</li>",
                    escape(&event.event_type),
                    event.state,
                    escape(&event.aggregate_id),
                    escape(&event.event_id)
                )
            })
            .collect::<String>();
        format!(
            "<section class='card section source-change stale'><div class='split'><div><p class='eyebrow'>Continuous validity</p><h2>Source successor impact</h2></div><span class='badge warning'>Stale after source change</span></div><p>A governed subscription snapshot was superseded after publication. Historical SQL, aggregate results, hashes, review, and replay evidence remain immutable.</p><div class='columns'><div><h3>Claim transitions</h3><ul class='checks'>{}</ul></div><div><h3>Durable revalidation jobs</h3><ul class='checks'>{}</ul></div></div><h3>Outbox records</h3><ul class='checks'>{}</ul></section>",
            stale,
            empty_state(jobs, "No revalidation jobs were recorded."),
            empty_state(outbox, "No invalidation outbox records were recorded.")
        )
    };
    let replay_key = crate::domain::new_id("ui_replay");
    let local_identity = state
        .demo_sessions
        .as_ref()
        .map(|_| {
            format!(
                "<p class='identity'>Local demo identity: <strong>{}</strong> · switch roles from the <a href='/'>workspace</a>.</p>",
                escape(&identity.subject_id)
            )
        })
        .unwrap_or_default();
    let mut body = local_identity;
    body.push_str(&format!(
        "<section class='boundary-bar'><span><b>Deployment</b> {}</span><span><b>Data execution</b> {}</span><span><b>Model route</b> {}</span><span><b>Allowed egress</b> {}</span><span><b>Telemetry</b> {}</span></section>",
        escape(boundary.deployment),
        escape(boundary.data_execution),
        escape(boundary.model_route),
        escape(&allowed_egress),
        escape(boundary.external_telemetry)
    ));
    body.push_str(&format!(
        "<section class='hero compact'><p class='eyebrow'>{} · {}</p><h1>{}</h1><p>{}</p><div class='status-line'><span class='badge'>Proposed by Gemma 4</span><span class='badge'>Admitted by AMOS</span><span class='badge'>Executed outside the model</span><span class='badge'>Verified</span><span class='badge warning'>Lifecycle {:?}</span><span class='badge'>Publication {:?}</span></div></section>",
        escape(boundary.qualified_product_claim),
        escape(&analysis.artifact.artifact_id),
        escape(&analysis.artifact.title),
        escape(&analysis.transaction.request),
        analysis.transaction.state,
        analysis.artifact.publication_validity
    ));
    body.push_str(&format!(
        "<section class='card report'>{artifact_body}</section>"
    ));
    body.push_str(&source_change_panel);
    body.push_str(&format!(
        "<section class='card section'><p class='eyebrow'>01 · Material claims</p><h2>Open every assertion to its exact support</h2><p>{} durable dependency edges bind claims to executions, checks, governed memory, and source versions.</p>{}</section>",
        analysis.dependencies.len(),
        empty_state(claims, "No claims were committed.")
    ));
    body.push_str(&format!(
        "<section class='card section'><p class='eyebrow'>02 · What Gemma proposed</p><h2>Typed analytical plan and narrative calls</h2><p>The model proposed and narrated. It received no warehouse handle, credential, raw row, capability signature, or publication authority.</p>{}{}</section>",
        empty_state(proposed_steps, "No typed proposal steps were recorded."),
        empty_state(model_invocations, "No model invocation was recorded.")
    ));
    body.push_str(&format!(
        "<section class='card section'><p class='eyebrow'>03 · What AMOS allowed and ran</p><h2>Admission → narrow capability → external execution → verification</h2><dl class='facts'><div><dt>A-TXN</dt><dd>{}</dd></div><div><dt>Task</dt><dd>{} v{}</dd></div><div><dt>Risk</dt><dd>{:?}</dd></div><div><dt>Plan identity</dt><dd>{}</dd></div></dl>{}</section>",
        escape(&analysis.transaction.atxn_id),
        escape(&analysis.transaction.task_type),
        analysis.transaction.task_version,
        analysis.transaction.risk_class,
        escape(&analysis.plan.model_identity),
        empty_state(plan_steps, "No admitted plan steps were recorded.")
    ));
    body.push_str(&format!(
        "<section class='card section'><p class='eyebrow'>04 · What the model was allowed to see</p><h2>Permission-first governed context</h2><p>{} tokens across {} selected objects. The frozen manifest hash is {}.</p>{}</section>",
        analysis.manifest.token_count,
        analysis.manifest.selected_objects.len(),
        escape(&crate::domain::content_hash(&analysis.manifest)?),
        empty_state(selected_context, "No governed context was selected.")
    ));
    body.push_str(&format!(
        "<section class='card section protected'><p class='eyebrow'>05 · Protected from the model</p><h2>Policy-withheld data and authority</h2><div class='columns'><div><h3>Never sent</h3><ul class='checks'><li>Raw warehouse rows</li><li>Warehouse paths and credentials</li><li>Capability tokens and signing keys</li><li>Restricted memory content</li><li>Publication authority</li></ul></div><div><h3>Blocked schema fields</h3><p>{}</p><h3>Safe omission summary</h3><ul class='checks'>{}</ul></div></div></section>",
        escape(&blocked_fields),
        empty_state(
            omissions,
            "No optional governed role was omitted; policy-restricted categories still remained outside the manifest."
        )
    ));
    body.push_str(&format!(
        "<section class='columns section'><section class='card'><p class='eyebrow'>06 · Verified facts</p><h2>Canonical fact catalog</h2>{}</section><section class='card'><p class='eyebrow'>Replay contract</p><h2>Durable reproducibility</h2><dl class='facts'><div><dt>Replay level</dt><dd>{}</dd></div><div><dt>Manifest</dt><dd>{}</dd></div><div><dt>Plan</dt><dd>{}</dd></div><div><dt>Expected artifact hash</dt><dd>{}</dd></div></dl><form method='post' action='/ui/artifacts/{}/replay'><input type='hidden' name='idempotency_key' value='{}'><button>Replay into a new A-TXN</button></form></section></section>",
        facts,
        analysis.replay.replay_level,
        escape(&analysis.replay.manifest_id),
        escape(&analysis.replay.plan_id),
        escape(&analysis.replay.expected_artifact_hash),
        escape(&analysis.artifact.artifact_id),
        replay_key
    ));
    body.push_str(&format!(
        "<section class='card section'><p class='eyebrow'>07 · Immutable activity</p><h2>Audit trail</h2><p>Policy epoch {} · refresh-safe, append-only control evidence.</p><ul class='checks'>{}</ul></section>",
        analysis.transaction.policy_epoch,
        empty_state(audit, "No audit events were recorded for this analysis.")
    ));
    Ok(Html(page(&analysis.artifact.title, &body)))
}

async fn claim_evidence(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Html<String>> {
    let lookup_identity = identity.clone();
    let claim_id = id.clone();
    let evidence = state
        .runtime
        .execute_blocking(move |runtime| runtime.claim_evidence_view(&lookup_identity, &claim_id))
        .await?;
    let claim = &evidence.claim;
    let supporting_executions = &evidence.executions;
    let execution_cards = supporting_executions
        .iter()
        .map(|execution| {
            let step = evidence
                .plan
                .steps
                .iter()
                .find(|step| step.step_id == execution.step_id);
            let sql = step
                .and_then(|step| step.parameters.get("sql"))
                .and_then(Value::as_str)
                .unwrap_or("No SQL parameter");
            let capability = step
                .map(|step| {
                    format!(
                        "{} · source {} · limits {}s/{} rows/{} bytes",
                        escape(&step.tool),
                        escape(&step.source_id),
                        step.limits.seconds,
                        step.limits.rows,
                        step.limits.bytes
                    )
                })
                .unwrap_or_else(|| "step metadata unavailable".into());
            Ok(format!(
                "<article><p class='eyebrow'>Exact governed query</p><pre>{}</pre><dl class='facts'><div><dt>Execution</dt><dd>{}</dd></div><div><dt>Capability</dt><dd>{}</dd></div><div><dt>Input versions</dt><dd>{}</dd></div><div><dt>Output hash</dt><dd>{}</dd></div></dl><p class='boundary-note'>Capability signature and credentials are redacted by design.</p><details open><summary>Verified aggregate result</summary><pre>{}</pre></details></article>",
                escape(sql),
                escape(&execution.execution_id),
                capability,
                escape(
                    &execution
                        .input_versions
                        .iter()
                        .map(|(source, version)| format!("{source}={version}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                escape(&execution.output_hash),
                pretty_json(&execution.output)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("");
    let checks = evidence
        .verifications
        .iter()
        .map(|verification| {
            let check_rows = verification
                .checks
                .iter()
                .map(|check| {
                    format!(
                        "<li><span class='badge'>{:?}</span> <strong>{}</strong>{}</li>",
                        check.outcome,
                        escape(&check.rule_id),
                        check
                            .message
                            .as_ref()
                            .map(|message| format!(" · {}", escape(message)))
                            .unwrap_or_default()
                    )
                })
                .collect::<String>();
            format!(
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>Profile {} v{} · input hash {}</p><ul class='checks'>{}</ul></article>",
                escape(&verification.verification_id),
                verification.outcome,
                escape(&verification.verifier_profile),
                verification.profile_version,
                escape(&verification.input_hash),
                check_rows
            )
        })
        .collect::<String>();
    let governed_support = evidence
        .dependencies
        .iter()
        .map(|edge| {
            let memory = (edge.to.endpoint_type == "memory")
                .then(|| {
                    evidence
                        .governed_objects
                        .iter()
                        .find(|object| object.object_id == edge.to.id)
                })
                .flatten();
            format!(
                "<li><strong>{}</strong> → {} {}{} </li>",
                escape(&edge.relation),
                escape(&edge.to.endpoint_type),
                escape(&edge.to.id),
                memory
                    .map(|object| format!(
                        " · {} · source version {}<details><summary>Governed excerpt</summary><pre>{}</pre></details>",
                        escape(&object.summary),
                        escape(&object.source_version),
                        pretty_json(&object.content)
                            .unwrap_or_else(|error| escape(&error.to_string()))
                    ))
                    .unwrap_or_else(|| {
                        edge.source_version
                            .as_ref()
                            .map(|version| format!(" · source version {}", escape(version)))
                            .unwrap_or_default()
                    })
            )
        })
        .collect::<String>();
    let model_lineage = evidence
        .model_invocations
        .iter()
        .map(|invocation| {
            format!(
                "<li><strong>{:?}</strong> · {}:{} · input {} · output {}</li>",
                invocation.purpose,
                escape(&invocation.provider),
                escape(&invocation.model),
                escape(&invocation.input_payload_hash),
                escape(invocation.output_hash.as_deref().unwrap_or("not recorded"))
            )
        })
        .collect::<String>();
    let audit = evidence
        .audit
        .iter()
        .map(|event| {
            format!(
                "<li><strong>{}</strong> · {} · {} · {}</li>",
                escape(&event.action),
                escape(&event.actor_id),
                escape(&event.outcome),
                event.created_at
            )
        })
        .collect::<String>();
    Ok(Html(page(
        "Claim evidence",
        &format!(
            "<p><a href='/analyses/{}'>← Back to analysis</a></p><section class='hero compact'><p class='eyebrow'>Claim evidence · {}</p><h1>{}</h1><p>Every support shown here is durable, tenant-scoped, policy-visible, and bound by content hash.</p><div class='status-line'><span class='badge'>{:?}</span><span class='badge'>{:?}</span><span class='badge'>{:?}</span></div></section><section class='card above-fold'><p class='eyebrow'>Query and result</p><h2>Direct computational support</h2>{}</section><section class='columns section'><section class='card'><p class='eyebrow'>Verifier</p><h2>Machine checks</h2>{}</section><section class='card'><p class='eyebrow'>Governance</p><h2>Metric, schema, data state, and provenance</h2><ul class='checks'>{}</ul></section></section><section class='columns section'><section class='card'><p class='eyebrow'>Model lineage</p><h2>Recorded proposal and narration hashes</h2><ul class='checks'>{}</ul></section><section class='card'><p class='eyebrow'>Claim payload</p><h2>Typed fact or judgment</h2><pre>{}</pre></section></section><section class='card section'><p class='eyebrow'>Human and system decisions</p><h2>Append-only audit evidence</h2><ul class='checks'>{}</ul></section>",
            escape(&claim.artifact_id),
            escape(&claim.claim_id),
            escape(&claim.text),
            claim.semantic_validity,
            claim.publication_validity,
            claim.review_state,
            empty_state(
                execution_cards,
                "This claim has no computational execution support."
            ),
            empty_state(checks, "No verifier records support this claim."),
            empty_state(governed_support, "No dependency edges were committed."),
            empty_state(model_lineage, "No model lineage was recorded."),
            pretty_json(&claim.payload)?,
            empty_state(audit, "No audit events were recorded.")
        ),
    )))
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
                "<article><div class='split'><strong>{}</strong><span class='badge'>{:?}</span></div><p>{}</p><small>A-TXN {} · {}</small><br><a class='text-link' href='/analyses/{}'>Open analysis and evidence →</a></article>",
                escape(&artifact.title),
                artifact.publication_validity,
                escape(&artifact.artifact_type),
                escape(&artifact.atxn_id),
                artifact.created_at,
                escape(&artifact.artifact_id)
            )
        })
        .collect::<String>();
    let task_key = crate::domain::new_id("ui_task");
    let boundary = state.runtime.privacy_boundary()?;
    let allowed_egress = if boundary.public_egress_allowlist.is_empty() {
        "none".into()
    } else {
        boundary.public_egress_allowlist.join(", ")
    };
    let identity_switch = state
        .demo_sessions
        .as_ref()
        .map(|_| {
            format!(
                "<section class='demo-session'><div><b>Local demo identity</b><small>Server-side session · HttpOnly · SameSite=Strict</small></div><form method='post' action='/demo/identity'><label class='sr-only' for='demo-identity'>Identity</label><select id='demo-identity' name='identity'><option value='analyst_001'{}>Analyst</option><option value='reviewer_001'{}>Reviewer</option><option value='admin'{}>Administrator</option></select><button type='submit'>Switch</button></form></section>",
                selected_attr(&identity.subject_id, "analyst_001"),
                selected_attr(&identity.subject_id, "reviewer_001"),
                selected_attr(&identity.subject_id, "admin")
            )
        })
        .unwrap_or_default();
    let advisor_demo = state
        .demo_sessions
        .as_ref()
        .map(|_| {
            "<section class='card section advisor-launch'><div><p class='eyebrow'>New concept walkthrough</p><h2>Retail banking advisor demo</h2><p>Prepare a synthetic client conversation with a relationship timeline, peer-adoption chart, suitability signals, and evidence placeholders.</p></div><a class='button' href='/advisor-demo'>Open Advisor Workspace →</a></section>"
        })
        .unwrap_or_default();
    Ok(Html(page(
        "AMOS · Verified analysis",
        &format!(
            "{}<section class='boundary-bar'><span><b>Deployment</b> {}</span><span><b>Data execution</b> {}</span><span><b>Model route</b> {}</span><span><b>Allowed egress</b> {}</span><span><b>Telemetry</b> {}</span></section><section class='hero'><p class='eyebrow'>Subscription intelligence workspace</p><h1>Ask the question.<br><em>Trust the answer.</em></h1><p>AMOS verifies the metric, schema, data state, permissions, and support behind every material claim. The model proposes and narrates; AMOS authorizes, executes, verifies, and publishes.</p><p class='identity'>Signed in as <strong>{}</strong> · roles {}</p></section>{}<section class='card'><form method='post' action='/ui/tasks'><input type='hidden' name='idempotency_key' value='{}'><label for='request'>What do you need to know?</label><textarea id='request' name='request' required>Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?</textarea><button type='submit'>Run governed analysis →</button></form></section><section class='grid'><article><b>01</b><h2>Permission-first context</h2><p>Approved metrics, schemas, data state, and policies only.</p></article><article><b>02</b><h2>Claim-level evidence</h2><p>Exact query, aggregate result, checks, versions, and hashes.</p></article><article><b>03</b><h2>Human publication gate</h2><p>Material recommendations remain draft until an authorized review.</p></article></section><section class='card section'><p class='eyebrow'>Recent policy-visible work</p><h2>Analysis history</h2>{}</section>",
            identity_switch,
            escape(boundary.deployment),
            escape(boundary.data_execution),
            escape(boundary.model_route),
            escape(&allowed_egress),
            escape(boundary.external_telemetry),
            escape(&identity.subject_id),
            escape(
                &identity
                    .roles
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            advisor_demo,
            task_key,
            empty_state(
                recent,
                "No analyses have been admitted for this identity yet."
            )
        ),
    )))
}

const ADVISOR_DEMO_QUESTION: &str = "Tell me about this client and what should I sell to him";

#[derive(Debug, Deserialize)]
struct AdvisorDemoForm {
    request: String,
}

async fn advisor_demo_workspace(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Html<String>> {
    if state.demo_sessions.is_none() {
        return Err(AmosError::NotFound("advisor concept demo".into()));
    }
    Ok(Html(page(
        "AMOS · Advisor Workspace",
        &format!(
            r##"
<section class="concept-strip">
  <span><b>Concept demo</b> Scripted future-product preview</span>
  <span><b>Data</b> Synthetic client and cohort</span>
  <span><b>Action</b> No sale or enrollment occurs</span>
</section>
<section class="advisor-hero">
  <div>
    <p class="eyebrow">Retail banking · Advisor workspace</p>
    <h1>Walk into every meeting <em>prepared.</em></h1>
    <p>AMOS assembles the client relationship, approved product signals, peer behavior, and conversation guardrails into one advisor-ready briefing.</p>
    <p class="identity">Signed in as <strong>{identity}</strong> · acting as authorized relationship advisor</p>
  </div>
  <aside class="client-card">
    <div class="client-avatar" aria-hidden="true">JL</div>
    <div>
      <span class="badge">Meeting in 5 minutes</span>
      <h2>Jordan Lee</h2>
      <p>Everyday Banking · Client since 2022</p>
    </div>
    <dl class="client-facts">
      <div><dt>Relationship</dt><dd>4 years</dd></div>
      <div><dt>Products</dt><dd>2 active</dd></div>
      <div><dt>Last contact</dt><dd>18 days ago</dd></div>
    </dl>
  </aside>
</section>
<section class="card advisor-question">
  <div class="split">
    <div>
      <p class="eyebrow">Ask AMOS</p>
      <h2>What do you want to know before the meeting?</h2>
    </div>
    <span class="badge">Gemma 4 · concept response</span>
  </div>
  <form method="post" action="/advisor-demo/analyze">
    <label for="advisor-request">Question</label>
    <textarea id="advisor-request" name="request" required>{question}</textarea>
    <div class="prompt-hints">
      <span>Relationship history</span>
      <span>Product fit</span>
      <span>Conversation opener</span>
      <span>Required disclosures</span>
    </div>
    <button type="submit">Prepare client briefing →</button>
  </form>
</section>
<section class="advisor-principles">
  <article><span>01</span><h3>Understand</h3><p>Bring the client’s recent relationship into one concise timeline.</p></article>
  <article><span>02</span><h3>Compare</h3><p>Use aggregate behavior from similar, currently eligible clients.</p></article>
  <article><span>03</span><h3>Discuss</h3><p>Recommend a suitable conversation—not an automatic product sale.</p></article>
</section>
"##,
            identity = escape(&identity.subject_id),
            question = ADVISOR_DEMO_QUESTION
        ),
    )))
}

async fn run_advisor_demo_form(
    State(state): State<AppState>,
    Extension(identity): Extension<Identity>,
    Form(form): Form<AdvisorDemoForm>,
) -> Result<Html<String>> {
    if state.demo_sessions.is_none() {
        return Err(AmosError::NotFound("advisor concept demo".into()));
    }
    let request = form.request.trim();
    if request.is_empty() || request.len() > 5_000 {
        return Err(AmosError::Validation(
            "advisor demo question must contain between 1 and 5000 characters".into(),
        ));
    }
    Ok(Html(page(
        "AMOS · Jordan Lee briefing",
        &advisor_demo_briefing(request, &identity),
    )))
}

fn advisor_demo_briefing(request: &str, identity: &Identity) -> String {
    format!(
        r##"
<section class="concept-strip">
  <span><b>Concept demo</b> Scripted future-product preview</span>
  <span><b>Client</b> Jordan Lee · synthetic</span>
  <span><b>Prepared for</b> {identity}</span>
</section>
<p class="back-link"><a href="/advisor-demo">← Ask another question</a></p>
<section class="briefing-hero">
  <div>
    <p class="eyebrow">AMOS client briefing · Prepared just now</p>
    <h1>Jordan’s next best <em>conversation.</em></h1>
    <p class="asked-question"><b>You asked:</b> “{request}”</p>
    <div class="status-line">
      <span class="badge">Proposed by Gemma 4</span>
      <span class="badge">Product rules checked</span>
      <span class="badge">Synthetic sources current</span>
      <span class="badge warning">Advisor confirmation required</span>
    </div>
  </div>
  <aside class="client-card compact-client">
    <div class="client-avatar" aria-hidden="true">JL</div>
    <div><p class="eyebrow">Client at a glance</p><h2>Jordan Lee</h2><p>Everyday Banking · Client since 2022</p></div>
    <dl class="client-facts">
      <div><dt>Deposits</dt><dd>$86.4K</dd></div>
      <div><dt>90-day low</dt><dd>$35.2K</dd></div>
      <div><dt>Active products</dt><dd>2</dd></div>
      <div><dt>Risk flags</dt><dd>None</dd></div>
    </dl>
  </aside>
</section>

<section class="recommendation-card">
  <div class="recommendation-rank">01</div>
  <div class="recommendation-copy">
    <p class="eyebrow">Recommended conversation</p>
    <h2>High-yield savings account</h2>
    <p>Jordan has maintained cash above normal monthly spending needs, uses direct deposit, and does not currently hold an interest-bearing deposit product.</p>
    <div class="fit-row">
      <span><b>Strong fit</b><small>Liquidity preserved</small></span>
      <span><b>3 verified signals</b><small>Across 2 synthetic sources</small></span>
      <span><b>1 question</b><small>Confirm near-term cash needs</small></span>
    </div>
  </div>
  <div class="confidence-ring" aria-label="Recommendation fit score 88 out of 100">
    <strong>88</strong><span>fit score</span>
  </div>
</section>

<section class="card section timeline-card">
  <div class="split">
    <div><p class="eyebrow">Relationship intelligence</p><h2>How Jordan’s needs evolved</h2></div>
    <span class="badge">4 years · 5 material events</span>
  </div>
  <div class="relationship-timeline" role="list" aria-label="Jordan Lee relationship timeline">
    <article role="listitem"><time>Mar 2022</time><span class="timeline-dot"></span><h3>Checking opened</h3><p>Primary checking relationship established.</p></article>
    <article role="listitem"><time>Aug 2022</time><span class="timeline-dot"></span><h3>Direct deposit</h3><p>Payroll deposits became recurring.</p></article>
    <article role="listitem"><time>Jun 2024</time><span class="timeline-dot"></span><h3>Rewards card</h3><p>Travel rewards card added and active.</p></article>
    <article role="listitem"><time>Nov 2025</time><span class="timeline-dot"></span><h3>Cash increased</h3><p>Average checking balance moved above $30K.</p></article>
    <article role="listitem" class="timeline-now"><time>Today</time><span class="timeline-dot"></span><h3>Opportunity</h3><p>No savings, money-market, or CD product.</p></article>
  </div>
</section>

<section class="advisor-dashboard section">
  <section class="card chart-card">
    <div class="split">
      <div><p class="eyebrow">Peer behavior</p><h2>What similar clients adopt</h2></div>
      <span class="badge">Eligible cohort · last 90 days</span>
    </div>
    <p class="chart-subtitle">Product adoption among 2,480 synthetic clients with a similar deposit profile.</p>
    <div class="bar-chart" role="img" aria-label="High-yield savings 24 percent, nine-month CD 16 percent, rewards card 11 percent, personal loan 5 percent">
      <div class="bar-row featured"><span>High-yield savings</span><i style="--value:24"></i><b>24%</b></div>
      <div class="bar-row"><span>9-month CD</span><i style="--value:16"></i><b>16%</b></div>
      <div class="bar-row"><span>Rewards card</span><i style="--value:11"></i><b>11%</b></div>
      <div class="bar-row"><span>Personal loan</span><i style="--value:5"></i><b>5%</b></div>
    </div>
    <p class="chart-note">Popularity is supporting context—not the recommendation rule.</p>
  </section>
  <section class="card chart-card">
    <div class="split">
      <div><p class="eyebrow">Liquidity signal</p><h2>Available balance trend</h2></div>
      <span class="badge">90 days</span>
    </div>
    <svg class="balance-chart" viewBox="0 0 620 270" role="img" aria-label="Jordan's available checking balance remained above 35 thousand dollars and ended at 42 thousand dollars">
      <defs>
        <linearGradient id="balance-fill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stop-color="#2f6b4d" stop-opacity=".28"/>
          <stop offset="100%" stop-color="#2f6b4d" stop-opacity=".02"/>
        </linearGradient>
      </defs>
      <line x1="54" y1="218" x2="590" y2="218" stroke="#d8ddd7"/>
      <line x1="54" y1="154" x2="590" y2="154" stroke="#e7eae5" stroke-dasharray="5 7"/>
      <line x1="54" y1="90" x2="590" y2="90" stroke="#e7eae5" stroke-dasharray="5 7"/>
      <path d="M54 190 L115 166 L176 178 L237 145 L298 154 L359 119 L420 128 L481 96 L542 108 L590 77 L590 218 L54 218 Z" fill="url(#balance-fill)"/>
      <polyline points="54,190 115,166 176,178 237,145 298,154 359,119 420,128 481,96 542,108 590,77" fill="none" stroke="#1f5a3d" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/>
      <circle cx="590" cy="77" r="7" fill="#c58b39" stroke="#fffefa" stroke-width="4"/>
      <text x="54" y="244">90 days ago</text><text x="524" y="244">Today</text>
      <text x="508" y="61" class="chart-value">$42.1K</text>
    </svg>
    <div class="signal-summary"><span><b>$35.2K</b><small>90-day low</small></span><span><b>+$8.7K</b><small>net change</small></span><span><b>0</b><small>overdraft events</small></span></div>
  </section>
</section>

<section class="advisor-dashboard section">
  <section class="card conversation-card">
    <p class="eyebrow">Suggested conversation opener</p>
    <blockquote>“You’ve consistently maintained funds above your normal monthly spending needs. Would you like to explore an account that may earn more interest while keeping your money accessible?”</blockquote>
    <h3>Ask before recommending</h3>
    <ul class="checks">
      <li>When do you expect to use these funds?</li>
      <li>How much access do you want to keep available?</li>
      <li>Would you prefer flexibility or a fixed term and rate?</li>
    </ul>
  </section>
  <section class="card alternatives-card">
    <p class="eyebrow">Alternative and exclusions</p>
    <h2>Consider a 9-month CD second</h2>
    <p>A CD may offer a stronger fixed return, but AMOS cannot establish suitability until Jordan confirms the expected timing of cash needs.</p>
    <div class="not-recommended"><span>Not prioritized</span><b>Another credit card</b><small>Jordan already holds an active rewards card with adequate limit utilization.</small></div>
    <div class="not-recommended"><span>Not indicated</span><b>Personal loan</b><small>No verified borrowing need or relevant client request was found.</small></div>
  </section>
</section>

<section class="card section guardrail-card">
  <div class="split">
    <div><p class="eyebrow">Advisor guardrails</p><h2>What must happen before the conversation</h2></div>
    <span class="badge warning">Human judgment required</span>
  </div>
  <div class="guardrail-grid">
    <article><b>Confirm suitability</b><p>Ask about liquidity needs and financial goals; eligibility alone is not suitability.</p></article>
    <article><b>Present current terms</b><p>Use the current rate sheet and provide the approved account disclosure.</p></article>
    <article><b>No automatic action</b><p>AMOS cannot open, enroll, or submit a product application for the client.</p></article>
    <article><b>Protected data excluded</b><p>Age, race, gender, disability, and other protected attributes were not used.</p></article>
  </div>
</section>

<section class="card section evidence-card">
  <div class="split">
    <div><p class="eyebrow">Why this answer is supportable</p><h2>Evidence and decision trace</h2></div>
    <span class="badge">Concept placeholders</span>
  </div>
  <div class="evidence-summary">
    <span><b>3</b><small>governed analyses</small></span>
    <span><b>7</b><small>policy checks passed</small></span>
    <span><b>2</b><small>versioned sources</small></span>
    <span><b>0</b><small>protected fields used</small></span>
  </div>
  <details>
    <summary>01 · Client relationship timeline</summary>
    <p class="boundary-note">Synthetic source: core_banking.client_product_events · snapshot 2026-07-27</p>
    <pre>SELECT event_date, event_type, product_name
FROM client_product_events
WHERE client_key = :authorized_client
ORDER BY event_date;</pre>
  </details>
  <details>
    <summary>02 · Product adoption among similar eligible clients</summary>
    <p class="boundary-note">Aggregate-only cohort. Minimum cohort size and product eligibility checks applied.</p>
    <pre>SELECT product_name, eligible_clients, adopted_clients, adoption_rate
FROM eligible_peer_product_adoption
WHERE segment = 'established_depositor'
  AND observation_window = 'last_90_days'
ORDER BY adoption_rate DESC;</pre>
  </details>
  <details>
    <summary>03 · Suitability signals and product rules</summary>
    <p class="boundary-note">Synthetic rulebook: retail_deposit_products:v7 · disclosure bundle: deposit_terms:v12</p>
    <pre>Verified signals
✓ stable_liquid_balance_90d
✓ no_interest_bearing_deposit_product
✓ recurring_direct_deposit

Still requires advisor confirmation
• expected_liquidity_horizon
• client_stated_goal
• acceptance_of_current_terms</pre>
  </details>
  <div class="model-boundary">
    <div><p class="eyebrow">Gemma 4 receives</p><p>Opaque client key, approved product definitions, aggregate cohort results, and verified suitability signals.</p></div>
    <div><p class="eyebrow">Gemma 4 never receives</p><p>Protected attributes, credentials, raw transaction descriptions, or authority to enroll the client.</p></div>
  </div>
</section>

<section class="briefing-footer">
  <div><p class="eyebrow">Ready for the meeting</p><h2>Use the evidence. Keep the judgment.</h2><p>This scripted preview shows the intended AMOS experience; it is not a live product recommendation.</p></div>
  <a class="button" href="/advisor-demo">Start over</a>
</section>
"##,
        request = escape(request),
        identity = escape(&identity.subject_id),
    )
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
            "<section class='hero compact'><p class='eyebrow'>Memory Studio</p><h1>Governed analytical memory</h1><p>Only policy-visible versions are shown. Search happens after tenant, status, type, time, and label filtering.</p><p class='identity'>Signed in as <strong>{}</strong></p></section><section class='columns'><section class='card'><h2>Permission-first search</h2><form method='post' action='/ui/memory/search'><label for='task_text'>Search governed memory</label><input id='task_text' name='task_text' required value='SMB logo churn definition'><button type='submit'>Search visible versions</button></form></section><section class='card'><h2>Record a user note</h2><p>Notes are permission-scoped, non-governing memory and cannot override approved definitions.</p><form method='post' action='/ui/memory/notes'><label for='logical_key'>Logical key</label><input id='logical_key' name='logical_key' required value='note:subscription-investigation'><label for='summary'>Summary</label><input id='summary' name='summary' required><label for='content'>Note</label><textarea id='content' name='content' required></textarea><button type='submit'>Record governed note</button></form></section></section><section class='card section'><h2>Active versions</h2>{}</section>",
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
        let analysis = state
            .runtime
            .execute_blocking(move |runtime| load_analysis(runtime, &detail_identity, &artifact_id))
            .await?;
        let claims = &analysis.claims;
        let freshness = analysis
            .manifest
            .selected_objects
            .iter()
            .find(|object| {
                object.content.get("role").and_then(Value::as_str) == Some("data_snapshot")
            })
            .and_then(|object| object.content.get("freshness_warning"))
            .and_then(Value::as_str)
            .unwrap_or("No source freshness warning was recorded.");
        let evidence_count =
            analysis.dependencies.len() + analysis.executions.len() + analysis.verifications.len();
        let claim_ids = claims
            .iter()
            .map(|claim| claim.claim_id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let claims_body = claims
            .iter()
            .map(|claim| {
                format!(
                    "<li><strong>{}</strong>: {} <span class='badge'>{:?}</span> <a href='/claims/{}'>Inspect evidence</a></li>",
                    escape(&claim.claim_type),
                    escape(&claim.text),
                    claim.review_state,
                    escape(&claim.claim_id)
                )
            })
            .collect::<String>();
        let review_key = crate::domain::new_id("ui_review");
        body.push_str(&format!(
            "<article><div class='split'><strong>{}</strong><span class='badge warning'>{:?}</span></div><p><b>Question:</b> {}</p><p>{} claims · {} evidence records · hash {}</p><p class='boundary-note'><b>Freshness:</b> {}</p><p><a class='text-link' href='/analyses/{}'>Open complete analysis record →</a></p><ul>{}</ul><details><summary>Record a consequential review</summary><form method='post' action='/ui/artifacts/{}/reviews'><input type='hidden' name='idempotency_key' value='{}'><input type='hidden' name='claim_ids' value='{}'><label for='decision-{}'>Decision</label><select id='decision-{}' name='decision'><option value='approve'>Approve and publish</option><option value='reject'>Reject publication</option><option value='correct'>Append correction</option></select><label for='comment-{}'>Reason</label><textarea id='comment-{}' name='comment' required></textarea><label for='correction-{}'>Structured correction (JSON; required for correction)</label><textarea id='correction-{}' name='correction' placeholder='{{&quot;causal_status&quot;:&quot;unproven&quot;}}'></textarea><label class='confirm'><input type='checkbox' name='confirmation' value='confirmed' required> I understand this appends a durable review and may publish, reject, or correct the artifact.</label><button type='submit'>Commit review</button></form></details></article>",
            escape(&analysis.artifact.title),
            analysis.artifact.publication_validity,
            escape(&analysis.transaction.request),
            claims.len(),
            evidence_count,
            escape(&analysis.artifact.content_hash),
            escape(freshness),
            escape(&analysis.artifact.artifact_id),
            claims_body,
            escape(&analysis.artifact.artifact_id),
            review_key,
            escape(&claim_ids),
            escape(&analysis.artifact.artifact_id),
            escape(&analysis.artifact.artifact_id),
            escape(&analysis.artifact.artifact_id),
            escape(&analysis.artifact.artifact_id),
            escape(&analysis.artifact.artifact_id),
            escape(&analysis.artifact.artifact_id)
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
fn selected_attr(value: &str, expected: &str) -> &'static str {
    if value == expected { " selected" } else { "" }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn pretty_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map(|body| escape(&body))
        .map_err(|error| AmosError::Storage(format!("evidence serialization failed: {error}")))
}
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{}</style></head><body><header><a href="/">AMOS</a><nav><a href="/">Workspace</a><a href="/memory">Memory Studio</a><a href="/reviews">Review Queue</a><a href="/operations">Operations</a></nav></header><main>{}</main></body></html>"#,
        escape(title),
        STYLE,
        body
    )
}
const STYLE: &str = r#":root{font-family:Inter,ui-sans-serif,system-ui;color:#17231c;background:#f6f4ed}*{box-sizing:border-box}body{margin:0}header{min-height:68px;padding:14px 5vw;display:flex;align-items:center;justify-content:space-between;gap:20px;border-bottom:1px solid #d9d9cf}header>a{font:700 22px Georgia;color:#17231c;text-decoration:none}nav{display:flex;flex-wrap:wrap;gap:18px}nav a{font-size:13px;color:#526159;text-decoration:none}a{color:#1f5a3d}a:focus-visible,button:focus-visible,input:focus-visible,textarea:focus-visible,select:focus-visible,summary:focus-visible{outline:3px solid #c58b39;outline-offset:3px}main{width:min(1120px,92vw);margin:48px auto}.hero{max-width:800px}.hero.compact h1{font-size:clamp(38px,5vw,62px)}.eyebrow{text-transform:uppercase;letter-spacing:.13em;color:#1f5a3d;font-size:11px;font-weight:800}h1{font:400 clamp(44px,7vw,76px)/1 Georgia;margin:18px 0}h1 em{color:#1f5a3d}h2{font:400 26px Georgia}h3{font:500 21px Georgia}.hero>p{color:#66736d;line-height:1.7;max-width:720px}.identity{padding:10px 14px;border-left:3px solid #1f5a3d;background:#eaf0e9}.card{min-width:0;background:#fffefa;border:1px solid #d9d9cf;border-radius:18px;padding:32px;box-shadow:0 18px 50px #1c2c2214}.card h1{font-size:38px}.section{margin-top:28px}.columns{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:24px;margin-top:24px}.columns>*{min-width:0}.split{display:flex;align-items:center;justify-content:space-between;gap:18px}.split form,.split button{margin:0}.boundary-bar{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));margin-bottom:34px;border:1px solid #cbd4cb;border-radius:12px;background:#eaf0e9}.boundary-bar span{padding:12px 14px;font-size:11px;color:#44534b}.boundary-bar span+span{border-left:1px solid #cbd4cb}.boundary-bar b{display:block;margin-bottom:4px;text-transform:uppercase;letter-spacing:.08em;color:#1f5a3d}.demo-session{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-bottom:18px;padding:12px 16px;border:1px dashed #9baa9f;border-radius:12px;background:#fffefa}.demo-session div small{display:block;margin-top:3px}.demo-session form{display:flex;align-items:center;gap:8px}.demo-session select{min-width:170px}.demo-session button{margin:0}.status-line{display:flex;flex-wrap:wrap;gap:8px;margin:20px 0}.boundary-note{padding:10px 12px;border-left:3px solid #c58b39;background:#fbf2df;font-size:13px}.report{overflow:hidden}.report svg{width:100%;height:auto}.step{padding:22px 0}.step+.step{border-top:2px solid #e7e6de}.checks{padding-left:20px;line-height:1.8}.checks li{overflow-wrap:anywhere}.text-link{display:inline-block;margin-top:10px;font-weight:700}.above-fold{border-color:#8ca28f}.sr-only{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}label{display:block;font-size:12px;font-weight:700;margin:12px 0 7px}textarea,select,input{width:100%;padding:14px;border:1px solid #bfc5bc;border-radius:10px;background:#faf9f4;color:#17231c}textarea{min-height:120px;font:20px Georgia}.confirm{display:flex;align-items:flex-start;gap:9px;font-weight:500;line-height:1.5}.confirm input{width:auto;margin-top:3px}button,.button{display:inline-block;margin-top:16px;padding:13px 18px;border:0;border-radius:10px;background:#17231c;color:#fff;font-weight:700;text-decoration:none;cursor:pointer}.grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));margin-top:35px;border-top:1px solid #d9d9cf}.grid article{padding:26px}.grid article+article{border-left:1px solid #d9d9cf}.metrics b{font:400 42px Georgia;color:#1f5a3d}article{min-width:0;padding:14px 0;border-bottom:1px solid #e7e6de;overflow-wrap:anywhere}article p,small,.empty{color:#66736d}.badge{display:inline-block;padding:5px 9px;border-radius:999px;background:#e8efe8;color:#1f5a3d;font-size:11px;font-weight:800}.badge.warning{background:#f5e8ca;color:#7d5617}.facts{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin:18px 0}.facts div{padding:12px;background:#f7f5ee;border-radius:8px}.facts dt{font-size:11px;text-transform:uppercase;letter-spacing:.08em;color:#66736d}.facts dd{margin:5px 0 0;overflow-wrap:anywhere}details{margin-top:15px;padding:14px;border:1px solid #d9d9cf;border-radius:10px}summary{cursor:pointer;font-weight:700}pre{white-space:pre-wrap;line-height:1.6;overflow-wrap:anywhere}.error{border-color:#a86666}.advisor-launch{display:flex;align-items:center;justify-content:space-between;gap:28px;border-color:#a9bbae;background:linear-gradient(120deg,#fffefa,#eaf0e9)}.advisor-launch p{max-width:700px}.advisor-launch .button{flex:none;margin:0}.concept-strip{display:grid;grid-template-columns:repeat(3,1fr);gap:0;margin-bottom:34px;border:1px solid #b8c8bc;border-radius:14px;background:#17231c;color:#dce6df;overflow:hidden}.concept-strip span{padding:13px 16px;font-size:11px}.concept-strip span+span{border-left:1px solid #405149}.concept-strip b{display:block;margin-bottom:4px;color:#fff;text-transform:uppercase;letter-spacing:.09em}.advisor-hero,.briefing-hero{display:grid;grid-template-columns:minmax(0,1.25fr) minmax(320px,.75fr);gap:44px;align-items:center;margin:30px 0 38px}.advisor-hero>div>p,.briefing-hero>div>p{max-width:720px;color:#66736d;line-height:1.7}.advisor-hero h1,.briefing-hero h1{font-size:clamp(46px,6vw,72px)}.client-card{display:grid;grid-template-columns:auto 1fr;gap:18px;padding:26px;border:1px solid #cdd7ce;border-radius:20px;background:#fffefa;box-shadow:0 18px 50px #1c2c2214}.client-card h2{margin:9px 0 4px}.client-card p{margin:0;color:#66736d}.client-avatar{display:grid;place-items:center;width:58px;height:58px;border-radius:50%;background:#1f5a3d;color:#fff;font:700 20px Georgia}.client-facts{display:grid;grid-template-columns:repeat(3,1fr);grid-column:1/-1;gap:8px;margin:5px 0 0}.client-facts div{padding:11px;background:#f2f5ef;border-radius:9px}.client-facts dt{font-size:10px;text-transform:uppercase;color:#66736d}.client-facts dd{margin:5px 0 0;font-weight:800}.compact-client .client-facts{grid-template-columns:repeat(2,1fr)}.advisor-question{border-color:#99aa9d}.advisor-question textarea{min-height:108px}.prompt-hints{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}.prompt-hints span{padding:6px 9px;border:1px solid #d9d9cf;border-radius:999px;color:#66736d;font-size:11px}.advisor-principles{display:grid;grid-template-columns:repeat(3,1fr);gap:0;margin-top:35px;border-top:1px solid #d9d9cf}.advisor-principles article{padding:26px}.advisor-principles article+article{border-left:1px solid #d9d9cf}.advisor-principles span{color:#c58b39;font-weight:800}.advisor-principles h3{margin:8px 0}.back-link{margin-bottom:24px}.asked-question{padding:12px 14px;border-left:3px solid #1f5a3d;background:#eaf0e9}.recommendation-card{display:grid;grid-template-columns:auto 1fr auto;gap:24px;align-items:center;padding:32px;border-radius:20px;background:#173e2b;color:#fff;box-shadow:0 22px 60px #17231c2e}.recommendation-card .eyebrow,.recommendation-card p,.recommendation-card small{color:#cfddd3}.recommendation-card h2{margin:8px 0 10px;font-size:34px}.recommendation-rank{align-self:start;color:#99b5a2;font:400 54px Georgia}.fit-row{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin-top:22px}.fit-row span{padding:12px;border:1px solid #ffffff24;border-radius:10px}.fit-row b,.fit-row small{display:block}.fit-row small{margin-top:5px}.confidence-ring{display:grid;place-items:center;align-content:center;width:118px;height:118px;border:7px solid #c58b39;border-left-color:#ffffff2b;border-radius:50%;text-align:center}.confidence-ring strong{font:400 38px Georgia}.confidence-ring span{font-size:10px;text-transform:uppercase;letter-spacing:.09em;color:#cfddd3}.relationship-timeline{display:grid;grid-template-columns:repeat(5,1fr);position:relative;margin-top:34px}.relationship-timeline:before{content:"";position:absolute;left:5%;right:5%;top:44px;height:2px;background:#cbd4cb}.relationship-timeline article{position:relative;padding:0 15px;border:0}.relationship-timeline time{display:block;height:28px;color:#66736d;font-size:11px;font-weight:800}.timeline-dot{display:block;position:relative;z-index:1;width:15px;height:15px;margin:9px 0 17px;border:4px solid #fffefa;border-radius:50%;background:#1f5a3d;box-shadow:0 0 0 2px #1f5a3d}.timeline-now .timeline-dot{background:#c58b39;box-shadow:0 0 0 2px #c58b39}.relationship-timeline h3{margin:0 0 8px;font-size:17px}.relationship-timeline p{font-size:12px;line-height:1.55}.advisor-dashboard{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:24px}.chart-subtitle,.chart-note{font-size:12px;line-height:1.6}.bar-chart{display:grid;gap:17px;margin:28px 0}.bar-row{display:grid;grid-template-columns:130px 1fr 40px;gap:12px;align-items:center;font-size:12px}.bar-row i{height:18px;border-radius:5px;background:linear-gradient(90deg,#76917e,#9fb2a3);width:calc(var(--value)*3.1%);min-width:8px}.bar-row.featured{font-weight:800}.bar-row.featured i{background:linear-gradient(90deg,#1f5a3d,#4a8063)}.bar-row b{text-align:right}.balance-chart{width:100%;height:auto;margin-top:10px}.balance-chart text{fill:#66736d;font:12px Inter,sans-serif}.balance-chart .chart-value{fill:#17231c;font:700 18px Georgia}.signal-summary,.evidence-summary{display:grid;grid-template-columns:repeat(3,1fr);gap:9px}.signal-summary span,.evidence-summary span{padding:11px;border-radius:9px;background:#f2f5ef}.signal-summary b,.signal-summary small,.evidence-summary b,.evidence-summary small{display:block}.signal-summary small,.evidence-summary small{margin-top:4px}.conversation-card blockquote{margin:24px 0;padding:18px 22px;border-left:4px solid #c58b39;background:#fbf2df;font:400 24px/1.4 Georgia}.alternatives-card h2{margin-bottom:8px}.not-recommended{display:grid;gap:5px;margin-top:14px;padding:13px;border:1px solid #e2e1d9;border-radius:10px}.not-recommended span{color:#7d5617;font-size:10px;font-weight:800;text-transform:uppercase;letter-spacing:.08em}.guardrail-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-top:24px}.guardrail-grid article{padding:17px;border:1px solid #e0e2dc;border-radius:12px;background:#f9f8f2}.guardrail-grid article p{font-size:12px;line-height:1.55}.evidence-summary{grid-template-columns:repeat(4,1fr);margin:24px 0}.evidence-summary span{text-align:center}.evidence-summary b{font:400 32px Georgia;color:#1f5a3d}.evidence-card details{background:#fbfaf5}.model-boundary{display:grid;grid-template-columns:repeat(2,1fr);gap:1px;margin-top:22px;border:1px solid #d9d9cf;border-radius:12px;overflow:hidden;background:#d9d9cf}.model-boundary div{padding:18px;background:#f6f4ed}.model-boundary p{font-size:12px;line-height:1.6}.briefing-footer{display:flex;align-items:center;justify-content:space-between;gap:24px;margin:40px 0 10px;padding:30px;border-radius:18px;background:#eaf0e9}.briefing-footer h2{margin:6px 0}.briefing-footer p{color:#66736d}.briefing-footer .button{flex:none;margin:0}@media(max-width:900px){.advisor-hero,.briefing-hero,.advisor-dashboard{grid-template-columns:1fr}.relationship-timeline{grid-template-columns:1fr}.relationship-timeline:before{left:22px;right:auto;top:5%;bottom:5%;width:2px;height:auto}.relationship-timeline article{display:grid;grid-template-columns:80px 24px 1fr;gap:9px;align-items:start}.relationship-timeline time{height:auto}.relationship-timeline .timeline-dot{margin:0}.relationship-timeline h3,.relationship-timeline p{grid-column:3}.guardrail-grid{grid-template-columns:repeat(2,1fr)}}@media(max-width:800px){header{align-items:flex-start;flex-direction:column}nav{gap:12px}main{margin:30px auto}.grid,.columns,.facts,.boundary-bar,.concept-strip,.advisor-principles,.fit-row,.model-boundary{grid-template-columns:minmax(0,1fr)}.grid article+article,.boundary-bar span+span,.concept-strip span+span,.advisor-principles article+article{border-left:0;border-top:1px solid #d9d9cf}.card{padding:22px}.split,.demo-session,.advisor-launch,.briefing-footer{align-items:flex-start;flex-direction:column}.demo-session form{width:100%}.recommendation-card{grid-template-columns:1fr}.confidence-ring{width:96px;height:96px}.guardrail-grid,.evidence-summary{grid-template-columns:1fr 1fr}}"#;
