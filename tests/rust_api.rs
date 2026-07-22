use std::collections::BTreeSet;
use std::process::Command;

use amos::{
    AmosRuntime, RuntimeConfig, api,
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

fn app() -> (TempDir, Router) {
    let root = TempDir::new().unwrap();
    let config = RuntimeConfig::demo(root.path());
    let store = Store::open(&config.control_db).unwrap();
    seed::seed_demo(&store, &config.warehouse_db).unwrap();
    let runtime = AmosRuntime::open(config).unwrap();
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
            "request":"Why did payment failures increase?",
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
    assert!(
        !serde_json::from_slice::<Value>(&body).unwrap()["dependencies"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let (status, body) = request(
        &app,
        "POST",
        "/v1/memory/search",
        "analyst_001",
        Some(json!({"task_text":"payment failure metric","max_items":10})),
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
            "request":"payment failure",
            "sql":"SELECT COUNT(*) AS attempts FROM payment_events WHERE event_time >= '2026-07-07T08:00:00Z' AND event_time < '2026-07-07T20:00:00Z' AND environment = 'production' AND is_test_account = 0"
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
            "request":"Why did payment failure rate increase over the last six hours?",
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
        assert!(!html.contains("name=\"identity\""), "{uri}");
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
        b"task_text=payment+failure+metric".to_vec(),
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
        b"logical_key=note%3Apayment-ui&summary=Observed+retry+pattern&content=Needs+review"
            .to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request(&app, "GET", "/v1/memory", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let note = serde_json::from_slice::<Vec<Value>>(&body)
        .unwrap()
        .into_iter()
        .find(|object| object["logical_key"] == "note:payment-ui")
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
async fn retention_api_erases_due_memory_and_revokes_dependent_claim_visibility() {
    let (_root, app) = app();
    let (status, body) = request(
        &app,
        "POST",
        "/v1/tasks",
        "analyst_001",
        Some(json!({
            "request":"Why did payment failure rate increase over the last six hours?",
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
                "request":"Why did payment failure rate increase?",
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
        "request":"Investigate payment failures",
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
            "request":"Review the payment failure evidence",
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
    assert!(root.path().join("data/payments.sqlite").exists());
}

#[test]
fn bundled_cli_run_requires_and_honors_a_caller_supplied_idempotency_key() {
    let root = TempDir::new().unwrap();
    let root_arg = root.path().to_str().unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args([
            "--demo",
            "--root",
            root_arg,
            "run",
            "--request",
            "Investigate payment failures",
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--idempotency-key"));

    let mut artifact_id = None;
    let mut durable_counts = None;
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_amos"))
            .args([
                "--demo",
                "--root",
                root_arg,
                "run",
                "--request",
                "Investigate payment failures",
                "--idempotency-key",
                "cli-task-repeat",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let run: RunResult = serde_json::from_slice(&output.stdout).unwrap();
        match artifact_id.as_ref() {
            Some(expected) => assert_eq!(expected, &run.artifact.artifact_id),
            None => artifact_id = Some(run.artifact.artifact_id),
        }
        let store = Store::open(root.path().join("data/amos.sqlite")).unwrap();
        let counts = (
            store.list_audit(seed::TENANT, 250).unwrap().len(),
            store.list_outbox(seed::TENANT, 500).unwrap().len(),
        );
        match durable_counts {
            Some(expected) => assert_eq!(expected, counts),
            None => durable_counts = Some(counts),
        }
    }
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
            "request":"Why did payment failures increase?",
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
            "request":"Why did payment failures increase?",
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
        !String::from_utf8(body)
            .unwrap()
            .contains("name=\"identity\"")
    );

    let (status, _) = request_raw(
        &app,
        "POST",
        "/ui/tasks",
        Some("Bearer analyst_001"),
        b"request=Why+did+payment+failures+increase%3F&idempotency_key=ui-identity-test&identity=reviewer_001".to_vec(),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

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
async fn memory_listing_applies_policy_permissions_before_serialization() {
    let (_root, app) = app();

    let (status, body) = request(&app, "GET", "/v1/memory", "analyst_001", None).await;
    assert_eq!(status, StatusCode::OK);
    let analyst_memory = serde_json::from_slice::<Vec<Value>>(&body).unwrap();
    assert!(
        analyst_memory
            .iter()
            .all(|object| object["logical_key"] != "analysis:processor_b_retry")
    );

    let (status, body) = request(&app, "GET", "/v1/memory", "admin", None).await;
    assert_eq!(status, StatusCode::OK);
    let admin_memory = serde_json::from_slice::<Vec<Value>>(&body).unwrap();
    assert!(
        admin_memory
            .iter()
            .any(|object| object["logical_key"] == "analysis:processor_b_retry")
    );
}
