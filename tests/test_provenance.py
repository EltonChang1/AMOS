from __future__ import annotations

from amos.agent.controller import run_amos_task
from amos.memory.models import User
from amos.memory.store import MemoryStore


def test_generated_claims_have_claim_level_provenance(seeded: None) -> None:
    result = run_amos_task(
        "Why did payment failure rate increase over the last six hours?",
        User(id="analyst_001", permissions=["analytics", "payments"]),
        provenance_level=3,
    )
    provenance = MemoryStore().list_claim_provenance(result.artifact_id)
    assert result.provenance_coverage == 1.0
    assert len(provenance) >= 3
    assert all(record.query_ids for record in provenance)
    assert all(record.memory_object_ids for record in provenance)

    rate = next(record for record in provenance if record.claim_id.endswith("_rate_increase"))
    concentration = next(record for record in provenance if record.claim_id.endswith("_concentration"))
    deployment = next(record for record in provenance if record.claim_id.endswith("_deployment"))
    dashboard = next(record for record in provenance if record.claim_id.endswith("_dashboard"))

    assert any(query_id.endswith("_summary") for query_id in rate.query_ids)
    assert any(query_id.endswith("_timeseries") for query_id in rate.query_ids)
    assert not any(query_id.endswith("_concentration") for query_id in rate.query_ids)
    assert [query_id for query_id in concentration.query_ids if query_id.endswith("_concentration")]
    assert not any(query_id.endswith("_summary") for query_id in concentration.query_ids)
    assert "memory_doc_payment_gateway_deploy_20260707" in deployment.document_refs
    assert "memory_feedback_avoid_overattribution" in deployment.document_refs
    assert any(query_id.endswith("_summary") for query_id in dashboard.query_ids)
    assert any(query_id.endswith("_concentration") for query_id in dashboard.query_ids)
    assert any(query_id.endswith("_timeseries") for query_id in dashboard.query_ids)
