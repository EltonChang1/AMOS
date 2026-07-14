use amos::{AmosRuntime, RuntimeConfig, api, domain::RunResult, seed, store::Store};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

fn app() -> (TempDir, Router) {
    let root = TempDir::new().unwrap();
    let config = RuntimeConfig::local(root.path());
    let store = Store::open(&config.control_db).unwrap();
    seed::seed_demo(&store, &config.warehouse_db).unwrap();
    let runtime = AmosRuntime::open(config).unwrap();
    (root, api::router(runtime))
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    identity: &str,
    payload: Option<Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {identity}"));
    let body = if let Some(payload) = payload {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&payload).unwrap())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
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
            "sql":"SELECT COUNT(*) AS attempts FROM payment_events WHERE environment = 'production' AND is_test_account = 0"
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
        None,
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
}
