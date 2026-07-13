from __future__ import annotations

from fastapi.testclient import TestClient

from amos.api.main import app
from amos.memory.models import ArtifactRecord, ClaimRecord
from amos.memory.store import MemoryStore
from amos.provenance.models import ClaimProvenance
from amos.tools.sql_templates import PAYMENT_WINDOW_END, PAYMENT_WINDOW_START


def test_memory_retrieve_uses_server_side_permissions(seeded: None) -> None:
    client = TestClient(app)

    response = client.post(
        "/memory/retrieve",
        headers={"X-AMOS-User": "analyst_001"},
        json={
            "task_text": "payment processor retry amplification incident",
            "required_types": ["prior_analysis"],
            "time_range": [PAYMENT_WINDOW_START, PAYMENT_WINDOW_END],
            "user_permissions": ["analytics", "payments", "sre"],
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "warning"
    assert body["user_id"] == "analyst_001"
    assert body["items"] == []
    assert "memory_prior_processor_retry_amplification" in body["filtered_permission_ids"]
    assert any("Ignored client-supplied permissions" in warning for warning in body["warnings"])


def test_memory_retrieve_allows_server_side_sre_identity(seeded: None) -> None:
    client = TestClient(app)

    response = client.post(
        "/memory/retrieve",
        headers={"X-AMOS-User": "sre_001"},
        json={
            "task_text": "payment processor retry amplification incident",
            "required_types": ["prior_analysis"],
            "time_range": [PAYMENT_WINDOW_START, PAYMENT_WINDOW_END],
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "pass"
    assert [item["id"] for item in body["items"]] == ["memory_prior_processor_retry_amplification"]
    assert body["filtered_permission_ids"] == []


def test_verify_sql_rejects_schema_and_metric_violations(seeded: None) -> None:
    client = TestClient(app)

    response = client.post(
        "/verify/sql",
        headers={"X-AMOS-User": "analyst_001"},
        json={
            "task_text": "Why did payment failure rate increase over the last six hours?",
            "sql": (
                "SELECT failure_reason FROM payment_events "
                f"WHERE event_time >= TIMESTAMP '{PAYMENT_WINDOW_START}'"
            ),
        },
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] == "reject"
    assert any("failure_reason" in error for error in body["errors"])
    assert "memory_schema_payment_events_v2" in body["used_memory_ids"]


def test_task_run_returns_product_contract(seeded: None) -> None:
    client = TestClient(app)

    response = client.post(
        "/tasks/run",
        headers={"X-AMOS-User": "analyst_001"},
        json={"request": "Why did payment failure rate increase over the last six hours?"},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["status"] in ["pass", "warning"]
    assert body["run_id"].startswith("run_")
    assert body["user_id"] == "analyst_001"
    assert body["tenant_id"] == "tenant_default"
    assert body["artifact_id"].startswith("report_")
    assert body["replay_package_id"].startswith("replay_")
    assert body["result"]["provenance_coverage"] >= 0.95


def test_product_home_and_artifact_workspace(seeded: None) -> None:
    client = TestClient(app)

    home = client.get("/")
    assert home.status_code == 200
    assert "Ask the question" in home.text
    assert "/static/app.js" in home.text

    run = client.post(
        "/tasks/run",
        headers={"X-AMOS-User": "analyst_001"},
        json={"request": "Why did payment failure rate increase over the last six hours?"},
    ).json()
    artifact_id = run["artifact_id"]

    detail_response = client.get(
        f"/artifacts/{artifact_id}",
        headers={"X-AMOS-User": "analyst_001"},
    )
    assert detail_response.status_code == 200
    detail = detail_response.json()
    assert detail["artifact"]["artifact_id"] == artifact_id
    assert "Payment failure rate increased" in detail["report_markdown"]
    assert detail["provenance_coverage"] >= 0.95
    assert len(detail["claims"]) >= 4
    assert len(detail["citations"]) == len(detail["claims"])
    assert len(detail["chart_urls"]) == 1

    chart_response = client.get(
        detail["chart_urls"][0],
        headers={"X-AMOS-User": "analyst_001"},
    )
    assert chart_response.status_code == 200
    assert chart_response.headers["content-type"] == "image/png"

    history = client.get(
        "/artifacts?limit=10",
        headers={"X-AMOS-User": "analyst_001"},
    ).json()
    assert artifact_id in [artifact["artifact_id"] for artifact in history["artifacts"]]


def test_artifact_workspace_is_scoped_to_owner(seeded: None) -> None:
    client = TestClient(app)
    run = client.post(
        "/tasks/run",
        headers={"X-AMOS-User": "analyst_001"},
        json={"request": "Why did payment failure rate increase over the last six hours?"},
    ).json()

    detail = client.get(
        f"/artifacts/{run['artifact_id']}",
        headers={"X-AMOS-User": "sre_001"},
    ).json()
    assert detail["status"] == "reject"
    assert detail["artifact"] is None


def test_reviewer_approved_feedback_requires_reviewer_identity(seeded: None) -> None:
    client = TestClient(app)
    response = client.post(
        "/memory/feedback",
        headers={"X-AMOS-User": "analyst_001"},
        json={
            "artifact_id": "report_test",
            "reviewer_role": "analyst",
            "feedback": "Treat the cause as pending review.",
            "authority": "reviewer_approved",
        },
    )

    assert response.status_code == 403
    assert "reviewer identity" in response.json()["detail"]


def test_artifact_provenance_redacts_restricted_memory(seeded: None) -> None:
    client = TestClient(app)
    store = MemoryStore()
    artifact_id = "report_api_redaction"
    claim_id = "claim_api_redaction"
    store.add_artifact(
        ArtifactRecord(
            artifact_id=artifact_id,
            artifact_type="report",
            path="/tmp/report_api_redaction.md",
            user_request="Investigate retry amplification.",
            task_plan_id="plan_api_redaction",
            created_by="sre_001",
        )
    )
    store.add_claim(
        ClaimRecord(
            claim_id=claim_id,
            artifact_id=artifact_id,
            claim_text="Restricted prior incident evidence supports the investigation.",
            claim_type="context",
        )
    )
    store.add_claim_provenance(
        ClaimProvenance(
            claim_id=claim_id,
            claim_text="Restricted prior incident evidence supports the investigation.",
            artifact_id=artifact_id,
            support=["memory_prior_processor_retry_amplification"],
            memory_object_ids=["memory_prior_processor_retry_amplification"],
            document_refs=["memory_prior_processor_retry_amplification"],
            semantic_state={
                "metric_definition_ids": [],
                "schema_ids": [],
                "stream_state_ids": [],
            },
        )
    )

    analyst_response = client.get(
        f"/artifacts/{artifact_id}/provenance",
        headers={"X-AMOS-User": "analyst_001"},
    )
    analyst_body = analyst_response.json()
    assert analyst_response.status_code == 200
    assert analyst_body["status"] == "warning"
    assert "memory_prior_processor_retry_amplification" not in str(analyst_body)
    assert analyst_body["claims"][0]["support"] == ["redacted_memory_1"]
    assert analyst_body["redactions"] == [
        {
            "redaction_id": "redacted_memory_1",
            "type": "prior_analysis",
            "sensitivity": "restricted",
            "reason": "insufficient_permissions",
        }
    ]

    sre_response = client.get(
        f"/artifacts/{artifact_id}/provenance",
        headers={"X-AMOS-User": "sre_001"},
    )
    sre_body = sre_response.json()
    assert sre_response.status_code == 200
    assert sre_body["status"] == "pass"
    assert sre_body["claims"][0]["support"] == ["memory_prior_processor_retry_amplification"]
