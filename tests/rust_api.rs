use std::process::Command;
use std::{collections::BTreeSet, sync::Arc};

use amos::{
    AmosRuntime, RuntimeConfig, api,
    auth::StaticIdentityProvider,
    domain::{Artifact, AuditEvent, PolicyVisibility, RunResult, new_id},
    seed,
    store::Store,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::WWW_AUTHENTICATE},
};
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn app() -> (TempDir, Router) {
    let root = TempDir::new().unwrap();
    let config = RuntimeConfig::demo(root.path()).unwrap();
    let store = Store::open(&config.control_db).unwrap();
    seed::seed_demo(&store, &config.warehouse_db).unwrap();
    let runtime = AmosRuntime::open_with_model(config, common::test_model()).unwrap();
    (root, api::demo_router(runtime))
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    identity: &str,
    payload: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let content_type = payload.as_ref().map(|_| "application/json");
    let body = payload
        .map(|value| serde_json::to_vec(&value).unwrap())
        .unwrap_or_default();
    request_raw(
        app,
        method,
        uri,
        Some(&format!("Bearer {identity}")),
        body,
        content_type,
    )
    .await
}

async fn request_raw(
    app: &Router,
    method: &str,
    uri: &str,
    authorization: Option<&str>,
    body: Vec<u8>,
    content_type: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(authorization) = authorization {
        builder = builder.header("authorization", authorization);
    }
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

#[tokio::test]
async fn versioned_api_exposes_the_complete_local_mvp_contract() {
    let (_root, app) = app();
    let (status, body) = request(&app, "GET", "/", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Ask the question")
    );

    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB logo churn increase this week?",
            "idempotency_key":"api-contract"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();

    let (status, _) = request(
        &app,
        "GET",
        &format!("/v1/tasks/{}", run.transaction.atxn_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let claim = &run.claims[0];
    let (status, body) = request(
        &app,
        "GET",
        &format!("/v1/claims/{}", claim.claim_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let evidence: Value = serde_json::from_slice(&body).unwrap();
    assert!(!evidence["dependencies"].as_array().unwrap().is_empty());
    assert!(!evidence["executions"].as_array().unwrap().is_empty());
    assert!(!evidence["verifications"].as_array().unwrap().is_empty());
    assert!(
        evidence["model_invocations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|invocation| {
                invocation.get("output_text").is_none()
                    && invocation.get("sanitized_input").is_none()
            })
    );

    let (status, body) = request(
        &app,
        "POST",
        "/v1/memory/search",
        "analyst_001",
        Some(json!({"task_text":"SMB logo churn metric","max_items":10})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !serde_json::from_slice::<Value>(&body).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let (status, body) = request(
        &app,
        "POST",
        "/v1/verify/sql",
        "analyst_001",
        Some(json!({
            "request":"SMB logo churn",
            "sql":"SELECT COUNT(*) AS eligible_accounts FROM subscription_events WHERE event_date >= '2026-07-13' AND event_date < '2026-07-27' AND segment = 'SMB' AND environment = 'production' AND is_test_account = 0"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        serde_json::from_slice::<Value>(&body).unwrap()["verification"]["outcome"],
        "reject"
    );

    let (status, _) = request(
        &app,
        "POST",
        &format!("/v1/replay/{}", run.artifact.artifact_id),
        "analyst_001",
        Some(json!({"idempotency_key":"api-contract-replay"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let review_claim = run
        .claims
        .iter()
        .find(|claim| claim.claim_type == "causal")
        .unwrap();
    let (status, _) = request(
        &app,
        "POST",
        "/v1/reviews",
        "reviewer_001",
        Some(json!({
            "idempotency_key":"api-review-correction",
            "artifact_id":run.artifact.artifact_id,
            "claim_ids":[review_claim.claim_id],
            "decision":"correct",
            "comment":"Correlation only.",
            "correction":{"causal_status":"unproven"},
            "authority":"reviewer_approved"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request(
        &app,
        "POST",
        &format!("/v1/artifacts/{}/revalidate", run.artifact.artifact_id),
        "reviewer_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = request(&app, "GET", "/v1/connectors/health", "analyst_001", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = request(&app, "GET", "/v1/connectors/health", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request(&app, "GET", "/v1/metrics", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_slice::<Value>(&body).unwrap()["task_succeeded"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

#[tokio::test]
async fn openapi_documents_every_versioned_route_and_public_security_boundary() {
    let (_root, app) = app();
    let (status, body) = request_raw(&app, "GET", "/v1/openapi.json", None, vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let document: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["security"][0]["bearerAuth"], json!([]));
    assert_eq!(
        document["paths"]["/v1/openapi.json"]["get"]["security"],
        json!([])
    );

    let documented = document["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "/v1/artifacts",
        "/v1/artifacts/page",
        "/v1/artifacts/{id}",
        "/v1/artifacts/{id}/replay",
        "/v1/artifacts/{id}/revalidate",
        "/v1/artifacts/{id}/reviews",
        "/v1/audit",
        "/v1/claims/{id}",
        "/v1/connectors/health",
        "/v1/jobs",
        "/v1/memory",
        "/v1/memory/search",
        "/v1/memory/{id}/supersede",
        "/v1/metrics",
        "/v1/openapi.json",
        "/v1/packs",
        "/v1/packs/{id}",
        "/v1/replay/{id}",
        "/v1/retention",
        "/v1/retention/memory/{id}/erase",
        "/v1/reviews",
        "/v1/source-events/process",
        "/v1/tasks",
        "/v1/tasks/{id}",
        "/v1/transactions/{id}",
        "/v1/verify/sql",
    ]);
    assert_eq!(documented, expected);

    for path in expected {
        for operation in document["paths"][path].as_object().unwrap().values() {
            assert!(operation["operationId"].is_string(), "{path}");
            assert!(operation["responses"].is_object(), "{path}");
        }
    }
}

#[tokio::test]
async fn four_product_surfaces_expose_the_governed_demo_and_safe_role_actions() {
    let (_root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
            "idempotency_key":"surface-walkthrough"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();

    for (uri, identity, expected) in [
        ("/", "analyst_001", "Recent policy-visible work"),
        ("/memory", "analyst_001", "Provenance"),
        ("/reviews", "reviewer_001", "Record a consequential review"),
        ("/operations", "admin", "Outbox delivery"),
    ] {
        let (status, body) = request(&app, "GET", uri, identity, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains(expected), "{uri}: {expected}");
        assert!(html.contains("<nav>"), "{uri}");
        if uri == "/" {
            assert!(html.contains("Local demo identity"));
            assert!(html.contains("action='/demo/identity'"));
        }
        if uri == "/reviews" {
            assert!(html.contains("Append correction"));
            assert!(html.contains("Structured correction"));
        }
        if uri == "/operations" {
            assert!(html.contains("Retention and privacy"));
            assert!(html.contains("I confirm this irreversible content erasure"));
        }
    }

    let (status, body) = request_raw(
        &app,
        "POST",
        "/ui/memory/search",
        Some("Bearer analyst_001"),
        b"task_text=SMB+logo+churn+metric".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Permission-first results")
    );

    let (status, _) = request_raw(
        &app,
        "POST",
        "/ui/memory/notes",
        Some("Bearer analyst_001"),
        b"logical_key=note%3Achurn-ui&summary=Observed+churn+pattern&content=Needs+review".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request(&app, "GET", "/v1/memory", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let note = serde_json::from_slice::<Vec<Value>>(&body)
        .unwrap()
        .into_iter()
        .find(|object| object["logical_key"] == "note:churn-ui")
        .unwrap();
    assert_eq!(note["authority"], "user_note");
    assert_eq!(note["governing"], false);

    let memory_id = &run.manifest.required_role_coverage["active_schema"][0];
    let retention_body = format!(
        "idempotency_key=surface-retention&target_type=memory&target_id={memory_id}&retained_until=2030-01-01T00%3A00%3A00Z&reason=Local+legal+review&confirmation=confirmed"
    );
    let (status, body) = request_raw(
        &app,
        "POST",
        "/ui/retention",
        Some("Bearer admin"),
        retention_body.into_bytes(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Retention updated")
    );

    let (status, body) = request_raw(
        &app,
        "POST",
        &format!("/ui/artifacts/{}/replay", run.artifact.artifact_id),
        Some("Bearer analyst_001"),
        b"idempotency_key=surface-replay".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("new fence"));

    let claim_ids = run
        .claims
        .iter()
        .map(|claim| claim.claim_id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let review_body = format!(
        "idempotency_key=surface-review&claim_ids={claim_ids}&decision=approve&comment=Evidence+reviewed&confirmation=confirmed"
    );
    let (status, body) = request_raw(
        &app,
        "POST",
        &format!("/ui/artifacts/{}/reviews", run.artifact.artifact_id),
        Some("Bearer reviewer_001"),
        review_body.into_bytes(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("Append-only review"));
    assert!(html.contains("Published"));

    let (status, body) = request_raw(
        &app,
        "POST",
        "/ui/source-events/process",
        Some("Bearer admin"),
        vec![],
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Source changes processed")
    );
}

#[tokio::test]
async fn advisor_concept_demo_accepts_a_question_and_renders_the_complete_synthetic_briefing() {
    let (_root, app) = app();
    let (status, body) = request_raw(&app, "GET", "/demo/login", None, vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Continue as Advisor")
    );

    let (status, body) = request(&app, "GET", "/advisor-demo", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    for expected in [
        "Retail banking · Advisor workspace",
        "Jordan Lee",
        "Tell me about this client and what should I sell to him",
        "Synthetic client and cohort",
        "No sale or enrollment occurs",
    ] {
        assert!(html.contains(expected), "{expected}");
    }

    let (status, body) = request_raw(
        &app,
        "POST",
        "/advisor-demo/analyze",
        Some("Bearer analyst_001"),
        b"request=Tell+me+about+this+client+and+what+should+I+sell+to+him%3F%3Cscript%3E".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    for expected in [
        "Jordan’s next best",
        "High-yield savings account",
        "How Jordan’s needs evolved",
        "What similar clients adopt",
        "Available balance trend",
        "Suggested conversation opener",
        "Alternative and exclusions",
        "Advisor guardrails",
        "Evidence and decision trace",
        "Protected data excluded",
        "This scripted preview shows the intended AMOS experience",
    ] {
        assert!(html.contains(expected), "{expected}");
    }
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
}

#[tokio::test]
async fn retention_api_erases_due_memory_and_revokes_dependent_claim_visibility() {
    let (_root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
            "idempotency_key":"erasure-task"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();
    let memory_id = run.manifest.required_role_coverage["active_schema"][0].clone();
    let command = json!({
        "target_type":"memory",
        "target_id":memory_id,
        "retained_until":"2020-01-01T00:00:00Z",
        "legal_hold":false,
        "reason":"approved privacy erasure",
        "idempotency_key":"retention-erasure-task"
    });
    for _ in 0..2 {
        let (status, _) = request(
            &app,
            "POST",
            "/v1/retention",
            "admin",
            Some(command.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    for _ in 0..2 {
        let (status, _) = request(
            &app,
            "POST",
            &format!("/v1/retention/memory/{memory_id}/erase"),
            "admin",
            Some(json!({"idempotency_key":"erase-memory-task"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, _) = request(
        &app,
        "GET",
        &format!("/v1/artifacts/{}", run.artifact.artifact_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn artifact_cursor_pagination_is_opaque_stable_and_fail_closed() {
    let (_root, app) = app();
    for key in ["cursor-task-a", "cursor-task-b"] {
        let (status, _) = request(
            &app,
            "POST",
            "/v1/tasks",
            "analyst_001",
            Some(json!({
                "request":"Why did SMB logo churn increase this week?",
                "idempotency_key":key
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, first_body) = request(
        &app,
        "GET",
        "/v1/artifacts/page?limit=1",
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().unwrap();
    assert!(!cursor.contains("art_"));
    let (status, second_body) = request(
        &app,
        "GET",
        &format!("/v1/artifacts/page?limit=1&cursor={cursor}"),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second: Value = serde_json::from_slice(&second_body).unwrap();
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_ne!(
        first["items"][0]["artifact_id"],
        second["items"][0]["artifact_id"]
    );

    let (status, _) = request(
        &app,
        "GET",
        "/v1/artifacts/page?cursor=not-base64!",
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn protected_api_and_ui_routes_fail_closed_without_valid_bearer_credentials() {
    let (_root, app) = app();

    let (status, _) = request_raw(&app, "GET", "/health", None, vec![], None).await;
    assert_eq!(status, StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/memory")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers().get(WWW_AUTHENTICATE).unwrap(), "Bearer");

    for uri in [
        "/",
        "/memory",
        "/reviews",
        "/operations",
        "/v1/artifacts",
        "/v1/memory",
        "/v1/jobs",
    ] {
        let (status, body) = request_raw(&app, "GET", uri, None, vec![], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "UNAUTHENTICATED"
        );
    }

    for authorization in [
        "Basic analyst_001",
        "Bearer",
        "Bearer analyst_001 extra",
        "Bearer unknown",
    ] {
        let (status, _) =
            request_raw(&app, "GET", "/v1/memory", Some(authorization), vec![], None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization}");
    }
}

#[tokio::test]
async fn health_reports_only_safe_runtime_boundary_and_compatibility_state() {
    let (_root, app) = app();
    let (status, body) = request_raw(&app, "GET", "/health", None, vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    let health: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(health["status"], "ok");
    assert_eq!(health["runtime"], "rust");
    assert_eq!(health["app_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        health["schema_version"],
        amos::store::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(health["model"]["provider"], "stub");
    assert_eq!(health["model"]["name"], "test-gemma");
    assert_eq!(health["model"]["route_class"], "local");
    assert_eq!(health["model"]["compatibility_probe_passed"], false);
    assert_eq!(health["warehouse"]["status"], "healthy");
    assert_eq!(health["external_telemetry"], "disabled");
    assert!(
        health["allowed_egress_hosts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let visible = String::from_utf8(body).unwrap();
    for forbidden in [
        "GEMINI_API_KEY",
        "Authorization",
        "Bearer ",
        "amos_demo_session",
        seed::WAREHOUSE_RAW_CANARY,
        seed::RESTRICTED_MEMORY_CANARY,
    ] {
        assert!(!visible.contains(forbidden), "{forbidden}");
    }
}

#[tokio::test]
async fn api_enforces_request_limits_idempotency_and_browser_security_headers() {
    let (_root, app) = app();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("x-request-id", "client-request-17")
                .header("x-correlation-id", "trace-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-request-id"], "client-request-17");
    assert_eq!(response.headers()["x-correlation-id"], "trace-9");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );

    let (status, _) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({"request":"missing command key"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let oversized = vec![b'x'; 1024 * 1024 + 1];
    let (status, _) = request_raw(
        &app,
        "POST",
        "/v1/tasks",
        Some("Bearer analyst_001"),
        oversized,
        Some("application/json"),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn task_admission_returns_a_stable_idempotency_conflict_for_a_changed_request() {
    let (_root, app) = app();
    let payload = json!({
        "request":"Investigate the SMB logo churn increase",
        "idempotency_key":"api-idempotency-conflict"
    });
    let (status, _) = request(&app, "POST", "/v1/tasks", "analyst_001", Some(payload)).await;
    assert_eq!(status, StatusCode::OK);

    let changed = json!({
        "request":"Investigate an unrelated request",
        "idempotency_key":"api-idempotency-conflict"
    });
    let (status, body) = request(&app, "POST", "/v1/tasks", "analyst_001", Some(changed)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );
}

#[tokio::test]
async fn review_mutations_are_idempotent_and_commit_one_feedback_job_and_event() {
    let (root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Review the SMB churn evidence",
            "idempotency_key":"review-idempotency-task"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();
    let claim_id = run
        .claims
        .iter()
        .find(|claim| claim.claim_type == "causal")
        .unwrap()
        .claim_id
        .clone();
    let payload = json!({
        "idempotency_key":"review-idempotency-command",
        "artifact_id":run.artifact.artifact_id,
        "claim_ids":[claim_id],
        "decision":"correct",
        "comment":"Keep the deployment statement explicitly correlational.",
        "correction":{"causal_status":"unproven"},
        "authority":"reviewer_approved"
    });
    let (status, first_body) = request(
        &app,
        "POST",
        "/v1/reviews",
        "reviewer_001",
        Some(payload.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first: Value = serde_json::from_slice(&first_body).unwrap();
    let (status, repeated_body) =
        request(&app, "POST", "/v1/reviews", "reviewer_001", Some(payload)).await;
    assert_eq!(status, StatusCode::OK);
    let repeated: Value = serde_json::from_slice(&repeated_body).unwrap();
    assert_eq!(
        first["review"]["review_id"],
        repeated["review"]["review_id"]
    );

    let changed = json!({
        "idempotency_key":"review-idempotency-command",
        "artifact_id":run.artifact.artifact_id,
        "claim_ids":[claim_id],
        "decision":"correct",
        "comment":"A different correction under the same key.",
        "correction":{"causal_status":"unsupported"},
        "authority":"reviewer_approved"
    });
    let (status, body) = request(&app, "POST", "/v1/reviews", "reviewer_001", Some(changed)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );

    let review_id = first["review"]["review_id"].as_str().unwrap();
    let store = Store::open(root.path().join("data/amos.sqlite")).unwrap();
    assert_eq!(
        store
            .list_active_memory(seed::TENANT)
            .unwrap()
            .iter()
            .filter(|memory| {
                memory.provenance_ref.as_deref() == Some(run.artifact.artifact_id.as_str())
                    && memory.source_version == review_id
            })
            .count(),
        1
    );
    assert_eq!(
        store
            .list_jobs(seed::TENANT, 100)
            .unwrap()
            .iter()
            .filter(|job| job.idempotency_key == format!("review/{review_id}/revalidate"))
            .count(),
        1
    );
    assert_eq!(
        store
            .list_outbox(seed::TENANT, 500)
            .unwrap()
            .iter()
            .filter(|event| {
                event.event_type == "review.appended" && event.aggregate_id == review_id
            })
            .count(),
        1
    );
}

#[test]
fn bundled_binary_requires_explicit_demo_mode_before_initializing_storage() {
    let root = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args(["--root", root.path().to_str().unwrap(), "seed"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("bundled binary has no production identity provider")
    );
    assert!(!root.path().join("data/amos.sqlite").exists());
}

#[test]
fn bundled_binary_initializes_demo_storage_only_when_demo_mode_is_explicit() {
    let root = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args(["--demo", "--root", root.path().to_str().unwrap(), "seed"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join("data/amos.sqlite").exists());
    assert!(root.path().join("data/warehouse.sqlite").exists());
}

#[test]
fn bundled_demo_server_rejects_non_loopback_binding_without_container_guard() {
    let root = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args([
            "--demo",
            "--root",
            root.path().to_str().unwrap(),
            "serve",
            "--bind",
            "0.0.0.0",
            "--port",
            "18081",
        ])
        .env_remove("AMOS_DEMO_LOOPBACK_PUBLISH")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("loopback binding"));
    assert!(!root.path().join("data/amos.sqlite").exists());
}

#[test]
fn bundled_cli_run_requires_a_key_and_fails_closed_without_a_model_provider() {
    let root = TempDir::new().unwrap();
    let root_arg = root.path().to_str().unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args([
            "--demo",
            "--root",
            root_arg,
            "run",
            "--request",
            "Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--idempotency-key"));

    let output = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args([
            "--demo",
            "--root",
            root_arg,
            "run",
            "--request",
            "Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
            "--idempotency-key",
            "cli-task-no-model",
        ])
        .env_remove("AMOS_MODEL_PROVIDER")
        .env_remove("GEMINI_API_KEY")
        .env_remove("GEMINI_API_KEY_FILE")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .to_ascii_lowercase()
            .contains("modelunavailable")
    );
    let store = Store::open(root.path().join("data/amos.sqlite")).unwrap();
    assert!(store.list_artifacts(seed::TENANT, 10).unwrap().is_empty());
}

#[test]
fn bundled_model_bootstrap_rejects_false_air_gap_claim_without_leaking_the_key() {
    let root = TempDir::new().unwrap();
    let secret = "never-print-this-gemini-key";
    let output = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args([
            "--demo",
            "--root",
            root.path().to_str().unwrap(),
            "model-probe",
        ])
        .env("AMOS_MODEL_PROVIDER", "gemma_api")
        .env("AMOS_MODEL_NAME", "gemma-4-26b-a4b-it")
        .env(
            "AMOS_MODEL_BASE_URL",
            "https://generativelanguage.googleapis.com/v1beta",
        )
        .env("AMOS_MODEL_ROUTE_CLASS", "approved_hosted_api")
        .env("AMOS_PRIVACY_PROFILE", "air_gapped")
        .env(
            "AMOS_ALLOWED_EGRESS_HOSTS",
            "generativelanguage.googleapis.com",
        )
        .env("GEMINI_API_KEY", secret)
        .env_remove("GEMINI_API_KEY_FILE")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let visible = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(visible.contains("air_gapped"));
    assert!(!visible.contains(secret));
    let store = Store::open(root.path().join("data/amos.sqlite")).unwrap();
    assert!(
        store
            .list_model_invocations(seed::TENANT, "any")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn transactions_artifacts_claims_and_replay_enforce_owner_and_policy_visibility() {
    let (root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB logo churn increase this week?",
            "idempotency_key":"owner-policy-contract"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();
    let artifact_id = &run.artifact.artifact_id;
    let claim_id = &run.claims[0].claim_id;

    let (status, _) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_002",
        Some(json!({
            "request":"Why did SMB logo churn increase this week?",
            "idempotency_key":"owner-policy-contract"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    for (method, uri) in [
        (
            "GET",
            format!("/v1/transactions/{}", run.transaction.atxn_id),
        ),
        ("GET", format!("/v1/artifacts/{artifact_id}")),
        ("GET", format!("/v1/claims/{claim_id}")),
        ("POST", format!("/v1/artifacts/{artifact_id}/replay")),
    ] {
        let (status, _) = request(&app, method, &uri, "analyst_002", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }

    let (status, body) = request(&app, "GET", "/v1/artifacts", "analyst_002", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_slice::<Vec<Artifact>>(&body)
            .unwrap()
            .is_empty()
    );

    let (status, _) = request(
        &app,
        "GET",
        &format!("/v1/artifacts/{artifact_id}"),
        "reviewer_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(
        &app,
        "POST",
        &format!("/v1/artifacts/{artifact_id}/replay"),
        "reviewer_001",
        Some(json!({"idempotency_key":"reviewer-policy-replay"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(
        &app,
        "POST",
        &format!("/v1/artifacts/{artifact_id}/revalidate"),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let store = Store::open(root.path().join("data/amos.sqlite")).unwrap();
    let expected_claim = store.get_claim(seed::TENANT, claim_id).unwrap().unwrap();
    let mut hidden_claim = expected_claim.clone();
    hidden_claim.policy_visibility = PolicyVisibility::Denied;
    store
        .commit_claim_validity_updates(
            &[expected_claim],
            &[hidden_claim],
            &AuditEvent {
                event_id: new_id("audit"),
                tenant_id: seed::TENANT.into(),
                actor_id: "test:policy".into(),
                action: "claim.policy_visibility.change".into(),
                target_type: "artifact".into(),
                target_id: artifact_id.clone(),
                request_id: None,
                atxn_id: None,
                outcome: "pass".into(),
                policy_epoch: 1,
                details: json!({"reason":"authorization test fixture"}),
                created_at: Utc::now(),
            },
            "policy.visibility_changed",
        )
        .unwrap();

    let (status, _) = request(
        &app,
        "POST",
        &format!("/v1/artifacts/{artifact_id}/reviews"),
        "reviewer_001",
        Some(json!({
            "idempotency_key":"hidden-claim-review",
            "claim_ids":[claim_id],
            "decision":"approve",
            "comment":"This policy-hidden claim must not be reviewable.",
            "correction":null,
            "authority":"reviewer_approved"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    for (method, uri) in [
        ("GET", format!("/v1/artifacts/{artifact_id}")),
        ("GET", format!("/v1/claims/{claim_id}")),
        ("POST", format!("/v1/artifacts/{artifact_id}/replay")),
    ] {
        let (status, _) = request(&app, method, &uri, "analyst_001", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    let (status, body) = request(&app, "GET", "/v1/artifacts", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_slice::<Vec<Artifact>>(&body)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn ui_uses_authenticated_identity_and_cannot_be_upgraded_by_form_fields() {
    let (_root, app) = app();
    let (status, body) = request(&app, "GET", "/", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Local demo identity")
    );

    let (status, _) = request_raw(
        &app,
        "POST",
        "/ui/tasks",
        Some("Bearer analyst_001"),
        b"request=Why+did+SMB+logo+churn+increase%3F&idempotency_key=ui-identity-test&identity=reviewer_001".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let (status, body) = request(&app, "GET", "/v1/artifacts", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let artifact = serde_json::from_slice::<Vec<Artifact>>(&body)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let (status, body) = request(
        &app,
        "GET",
        &format!("/v1/transactions/{}", artifact.atxn_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["subject_id"],
        "analyst_001"
    );

    let (status, _) = request(&app, "GET", "/reviews", "analyst_001", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = request(&app, "GET", "/operations", "analyst_001", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn demo_identity_switch_uses_an_opaque_server_session_and_is_absent_in_production() {
    let (_root, app) = app();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/demo/identity")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("identity=reviewer_001"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/");
    let set_cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(set_cookie.contains("amos_demo_session=demo_session_"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Strict"));
    assert!(!set_cookie.contains("reviewer_001"));
    let cookie = set_cookie.split(';').next().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("Signed in as <strong>reviewer_001</strong>"));

    let root = TempDir::new().unwrap();
    let config = RuntimeConfig::demo(root.path()).unwrap();
    let store = Store::open(&config.control_db).unwrap();
    seed::seed_demo(&store, &config.warehouse_db).unwrap();
    let runtime = AmosRuntime::open_with_model(config, common::test_model()).unwrap();
    let production = api::router(runtime, Arc::new(StaticIdentityProvider::demo()));
    let (status, _) = request_raw(
        &production,
        "POST",
        "/demo/identity",
        None,
        b"identity=admin".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = request_raw(
        &production,
        "POST",
        "/demo/source-change",
        Some("Bearer admin"),
        b"artifact_id=artifact&idempotency_key=source".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = request(&production, "GET", "/advisor-demo", "analyst_001", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = request_raw(&production, "GET", "/demo/login", None, vec![], None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = request(&production, "GET", "/", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !String::from_utf8(body)
            .unwrap()
            .contains("Local demo identity")
    );
}

#[tokio::test]
async fn subscription_ui_exposes_complete_evidence_and_published_review_without_raw_data() {
    let (_root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?",
            "idempotency_key":"m5-filmable-analysis"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();

    let (status, body) = request(
        &app,
        "GET",
        &format!("/analyses/{}", run.artifact.artifact_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    for expected in [
        "Permission-filtered model payload",
        "AMOS admitted",
        "Exact read-only SQL",
        "subscription_events",
        "Narrow capability issued",
        "AMOS verification checks",
        "Open complete claim evidence",
        "stub:test-gemma",
        "Protected from the model",
        "Blocked schema fields",
        "customer_email",
        "raw_support_note",
        "Invocation ID",
        "Latency",
        "Tokens",
        "Manifest / prompt hash",
        "0 raw warehouse rows",
    ] {
        assert!(html.contains(expected), "{expected}");
    }
    for forbidden in [seed::WAREHOUSE_RAW_CANARY, seed::RESTRICTED_MEMORY_CANARY] {
        assert!(!html.contains(forbidden), "{forbidden}");
    }

    let supported_claim = run
        .claims
        .iter()
        .find(|claim| !claim.support_execution_ids.is_empty())
        .unwrap();
    let (status, body) = request(
        &app,
        "GET",
        &format!("/claims/{}", supported_claim.claim_id),
        "analyst_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    for expected in [
        "Query and result",
        "Direct computational support",
        "Capability signature and credentials are redacted",
        "Verified aggregate result",
        "source version",
        "Model lineage",
        "Governed excerpt",
    ] {
        assert!(html.contains(expected), "{expected}");
    }

    let (status, body) = request(&app, "GET", "/reviews", "reviewer_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let review_html = String::from_utf8(body).unwrap();
    assert!(review_html.contains("<b>Question:</b>"));
    assert!(review_html.contains("evidence records"));
    assert!(review_html.contains("<b>Freshness:</b>"));
    assert!(review_html.contains("final day is incomplete"));

    let claim_ids = run
        .claims
        .iter()
        .map(|claim| claim.claim_id.clone())
        .collect::<Vec<_>>();
    let (status, body) = request(
        &app,
        "POST",
        &format!("/v1/artifacts/{}/reviews", run.artifact.artifact_id),
        "reviewer_001",
        Some(json!({
            "idempotency_key":"m5-publish",
            "claim_ids":claim_ids,
            "decision":"approve",
            "comment":"The evidence supports publication with the causal caveat intact.",
            "correction":null,
            "authority":"reviewer_approved"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let review: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(review["transaction"]["state"], "published");
    assert_eq!(
        review["artifact"]["publication_validity"],
        "valid_at_publication"
    );

    let (status, body) = request(
        &app,
        "GET",
        &format!("/analyses/{}", run.artifact.artifact_id),
        "reviewer_001",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    assert!(html.contains("Lifecycle Published"));
    assert!(html.contains("Publication ValidAtPublication"));

    let (status, body) = request(
        &app,
        "GET",
        &format!("/analyses/{}", run.artifact.artifact_id),
        "admin",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains("Receive updated snapshot")
    );
    let (status, _) = request_raw(
        &app,
        "POST",
        "/demo/source-change",
        Some("Bearer admin"),
        format!(
            "artifact_id={}&idempotency_key=m6-api-source-successor",
            run.artifact.artifact_id
        )
        .into_bytes(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, body) = request(
        &app,
        "GET",
        &format!("/analyses/{}", run.artifact.artifact_id),
        "admin",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let html = String::from_utf8(body).unwrap();
    for expected in [
        "Stale after source change",
        "Historical SQL, aggregate results, hashes, review, and replay evidence remain immutable",
        "Durable revalidation jobs",
        "claim.revalidate",
        "Outbox records",
        "invalidation.processed",
        "source.successor.receive",
    ] {
        assert!(html.contains(expected), "{expected}");
    }
}

#[tokio::test]
async fn memory_listing_applies_policy_permissions_before_serialization() {
    let (_root, app) = app();

    let (status, body) = request(&app, "GET", "/v1/memory", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let analyst_memory = serde_json::from_slice::<Vec<Value>>(&body).unwrap();
    assert!(
        analyst_memory
            .iter()
            .all(|object| object["logical_key"] != "analysis:churn_pipeline_incident")
    );

    let (status, body) = request(&app, "GET", "/v1/memory", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    let admin_memory = serde_json::from_slice::<Vec<Value>>(&body).unwrap();
    assert!(
        admin_memory
            .iter()
            .any(|object| object["logical_key"] == "analysis:churn_pipeline_incident")
    );
}

#[tokio::test]
async fn admin_can_list_inspect_and_install_analysis_packs() {
    let (_root, app) = app();

    let (status, _) = request(&app, "GET", "/v1/packs", "analyst_001", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = request(&app, "GET", "/v1/packs", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = serde_json::from_slice::<Value>(&body).unwrap();
    let items = listed["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|item| item["pack_id"] == "subscription_churn")
    );
    assert!(
        items
            .iter()
            .any(|item| item["pack_id"] == "payment_failure_churn")
    );

    let (status, body) = request(
        &app,
        "GET",
        "/v1/packs/payment_failure_churn",
        "admin",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pack = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(pack["task_type"], "payment_failure_churn_review");

    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did SMB payment failure churn increase this week?",
            "task_type":"payment_failure_churn_review",
            "idempotency_key":"api-payment-failure-pack"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run: RunResult = serde_json::from_slice(&body).unwrap();
    assert_eq!(run.transaction.task_type, "payment_failure_churn_review");

    let mut reinstall = pack.clone();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/packs",
        "admin",
        Some(json!({ "pack": reinstall })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let installed = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(installed["newly_installed"], false);
    assert_eq!(installed["pack_id"], "payment_failure_churn");

    reinstall["version"] = json!(2);
    let (status, body) = request(
        &app,
        "POST",
        "/v1/packs",
        "admin",
        Some(json!({ "pack": reinstall })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let upgraded = serde_json::from_slice::<Value>(&body).unwrap();
    assert_eq!(upgraded["newly_installed"], true);
    assert_eq!(upgraded["version"], 2);
}
