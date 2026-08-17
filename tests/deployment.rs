use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use amos::{
    auth::{HashedIdentityEntry, HashedIdentityManifest, HashedTokenIdentityProvider},
    deployment::{DeploymentMode, ServerConfig},
    domain::Identity,
};
use tempfile::TempDir;

fn configured_root() -> (TempDir, std::path::PathBuf, String, u16) {
    let root = TempDir::new().unwrap();
    let port = {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let secrets = root.path().join("secrets");
    fs::create_dir_all(&secrets).unwrap();
    fs::write(secrets.join("capability-key"), "17".repeat(32)).unwrap();
    let token = "customer-evaluation-analyst-token-with-enough-entropy".to_string();
    let identity = Identity {
        tenant_id: "tenant_demo".into(),
        subject_id: "customer_analyst".into(),
        roles: ["analyst".into()].into_iter().collect(),
        groups: Default::default(),
        permissions: ["analytics".into(), "payments".into()]
            .into_iter()
            .collect(),
        policy_attributes: Default::default(),
        policy_epoch: 1,
    };
    let manifest = HashedIdentityManifest {
        schema_version: 1,
        identities: vec![HashedIdentityEntry {
            token_sha256: HashedTokenIdentityProvider::token_hash(&token).unwrap(),
            identity,
        }],
    };
    fs::write(
        secrets.join("identities.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let config = ServerConfig {
        schema_version: 1,
        deployment_mode: DeploymentMode::CustomerEvaluation,
        acknowledge_local_reference_adapters: true,
        bind_address: "127.0.0.1".parse().unwrap(),
        port,
        public_base_url: format!("https://127.0.0.1:{port}"),
        control_db: root.path().join("data/control.sqlite"),
        warehouse_db: root.path().join("data/warehouse.sqlite"),
        object_root: root.path().join("objects"),
        capability_key_file: secrets.join("capability-key"),
        identities_file: secrets.join("identities.json"),
        toolbox_endpoint: None,
    };
    let config_path = root.path().join("server.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    (root, config_path, token, port)
}

#[test]
fn amosctl_validates_bootstraps_and_reports_status_without_overwriting() {
    let (root, config, _, _) = configured_root();
    let validate = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["validate", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    assert!(String::from_utf8_lossy(&validate.stdout).contains("\"status\": \"valid\""));

    let bootstrap = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["bootstrap-reference", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        bootstrap.status.success(),
        "{}",
        String::from_utf8_lossy(&bootstrap.stderr)
    );
    assert!(root.path().join("data/control.sqlite").is_file());
    assert!(root.path().join("data/warehouse.sqlite").is_file());

    let preflight = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["preflight", "--config"])
        .arg(&config)
        .arg("--require-initialized")
        .output()
        .unwrap();
    assert!(
        preflight.status.success(),
        "{}",
        String::from_utf8_lossy(&preflight.stderr)
    );

    let status = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["status", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("\"status\": \"initialized\""));

    let repeated = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["bootstrap-reference", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("refuses to overwrite"));
}

#[test]
fn configured_server_starts_and_serves_an_authenticated_task() {
    let (_root, config, token, port) = configured_root();
    let bootstrap = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["bootstrap-reference", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(bootstrap.status.success());

    let mut server = Command::new(env!("CARGO_BIN_EXE_amos"))
        .args(["--config"])
        .arg(&config)
        .arg("serve")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut ready = false;
    for _ in 0..50 {
        let health = Command::new(env!("CARGO_BIN_EXE_amosctl"))
            .args([
                "health",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--timeout-seconds",
                "1",
            ])
            .output()
            .unwrap();
        if health.status.success() {
            ready = true;
            break;
        }
        if server.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if !ready {
        let _ = server.kill();
        let _ = server.wait();
        panic!("configured AMOS server did not become healthy");
    }

    let body = serde_json::json!({
        "request": "Why did payment failures increase?",
        "idempotency_key": "configured-server-e2e"
    })
    .to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    stream
        .write_all(
            format!(
                "POST /v1/tasks HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    let _ = server.kill();
    let _ = server.wait();

    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("configured-server-e2e"));
    assert!(response.contains("needs_review"));
}

#[test]
fn amosctl_lists_builtins_and_validates_contract_only_tool_templates() {
    let list = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["tools", "list"])
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let list_body = String::from_utf8_lossy(&list.stdout);
    assert!(list_body.contains("sql.readonly.v1"));
    assert!(list_body.contains("stats.rate_comparison.v1"));
    assert!(list_body.contains("chart.timeseries.v1"));
    assert!(list_body.contains("spark.dataframe.aggregate.v1"));
    assert!(list_body.contains("polars.dataframe.aggregate.v1"));
    assert!(list_body.contains("presentation.pptx.v1"));

    let manifest_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tool-packs");
    let validate = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args(["tools", "validate", "--manifest"])
        .arg(manifest_root.join("templates/spark.dataframe.aggregate.v1.json"))
        .arg("--manifest")
        .arg(manifest_root.join("templates/r.statistics.v1.json"))
        .output()
        .unwrap();
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let validate_body = String::from_utf8_lossy(&validate.stdout);
    assert!(validate_body.contains("\"manifest_count\": 2"));
    assert!(validate_body.contains("spark.dataframe.aggregate.v1"));
    assert!(validate_body.contains("r.statistics.v1"));
    assert!(validate_body.contains("contract_only"));
}
