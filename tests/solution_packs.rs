use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use amos::{
    error::AmosError,
    solution_pack::{SignedSolutionPack, TRUST_STORE_SCHEMA_V1, TrustStore, TrustedPublisher},
};
use chrono::{TimeZone, Utc};
use ed25519_dalek::SigningKey;
use tempfile::tempdir;

fn repository_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn checked_in_solution_packs_have_valid_tenant_scoped_signatures() {
    let trust = TrustStore::load(repository_path(
        "solution-packs/trust/development-fixtures.json",
    ))
    .unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap();

    let bank = SignedSolutionPack::load(repository_path(
        "solution-packs/bank-weekly-liquidity.v1.json",
    ))
    .unwrap();
    let bank_verified = bank
        .verify_for_activation(&trust, "tenant_bank_fixture", "0.2.0", now)
        .unwrap();
    assert_eq!(bank_verified.workflow_id, "bank_weekly_liquidity_review");
    assert!(bank.manifest.bank_metadata.is_some());

    let payment = SignedSolutionPack::load(repository_path(
        "solution-packs/payment-health-regression.v1.json",
    ))
    .unwrap();
    let payment_verified = payment
        .verify_for_activation(&trust, "tenant_demo", "0.2.0", now)
        .unwrap();
    assert_eq!(payment_verified.workflow_id, "payment_health_regression");
    assert!(payment.manifest.bank_metadata.is_none());

    assert!(matches!(
        bank.verify_for_activation(&trust, "tenant_demo", "0.2.0", now),
        Err(AmosError::PermissionDenied(_))
    ));
    assert!(matches!(
        payment.verify_for_activation(&trust, "tenant_bank_fixture", "0.2.0", now),
        Err(AmosError::PermissionDenied(_))
    ));
}

#[test]
fn validation_cli_reports_pack_identity_without_private_key_material() {
    let output = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args([
            "solution-packs",
            "validate",
            "--pack",
            repository_path("solution-packs/bank-weekly-liquidity.v1.json")
                .to_str()
                .unwrap(),
            "--trust-store",
            repository_path("solution-packs/trust/development-fixtures.json")
                .to_str()
                .unwrap(),
            "--tenant",
            "tenant_bank_fixture",
            "--core-version",
            "0.2.0",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["status"], "valid");
    assert_eq!(body["packs"][0]["pack_id"], "amos.bank.weekly_liquidity.v1");
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .find("private_key")
            .is_none()
    );
}

#[test]
fn signing_cli_reads_private_material_from_stdin_and_writes_a_verifiable_copy() {
    let temporary = tempdir().unwrap();
    let unsigned_path = temporary.path().join("unsigned.json");
    let signed_path = temporary.path().join("signed.json");
    let mut unsigned = SignedSolutionPack::load(repository_path(
        "solution-packs/bank-weekly-liquidity.v1.json",
    ))
    .unwrap();
    unsigned.signatures.clear();
    std::fs::write(
        &unsigned_path,
        serde_json::to_vec_pretty(&unsigned).unwrap(),
    )
    .unwrap();

    let key_bytes = [9_u8; 32];
    let signing_key = SigningKey::from_bytes(&key_bytes);
    let mut child = Command::new(env!("CARGO_BIN_EXE_amosctl"))
        .args([
            "solution-packs",
            "sign",
            "--pack",
            unsigned_path.to_str().unwrap(),
            "--output",
            signed_path.to_str().unwrap(),
            "--key-id",
            "test-signing-key",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(hex::encode(key_bytes).as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&hex::encode(key_bytes)));

    let trust = TrustStore {
        schema_version: TRUST_STORE_SCHEMA_V1.into(),
        publishers: vec![TrustedPublisher {
            key_id: "test-signing-key".into(),
            public_key_hex: hex::encode(signing_key.verifying_key().to_bytes()),
            tenant_allowlist: BTreeSet::from(["tenant_bank_fixture".into()]),
        }],
    };
    SignedSolutionPack::load(&signed_path)
        .unwrap()
        .verify_for_activation(
            &trust,
            "tenant_bank_fixture",
            "0.2.0",
            Utc.with_ymd_and_hms(2026, 8, 21, 0, 0, 0).unwrap(),
        )
        .unwrap();
}
