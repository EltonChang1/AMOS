from __future__ import annotations

from pathlib import Path

from amos.agent.controller import run_amos_task
from amos.memory.models import User
from amos.provenance.replay import replay_artifact


def test_end_to_end_payment_failure_investigation(seeded: None) -> None:
    request = "Why did payment failure rate increase over the last six hours?"
    user = User(id="analyst_001", permissions=["analytics", "payments"])

    result = run_amos_task(request=request, user=user, provenance_level=3)

    assert result.artifact_id is not None
    assert result.verification_status in ["pass", "warning"]
    assert "memory_metric_payment_failure_rate_v3" in result.used_memory_ids
    assert "memory_schema_payment_events_v2" in result.used_memory_ids
    assert result.provenance_coverage >= 0.95
    assert result.replay_package_id is not None

    report_text = Path(result.report_path).read_text(encoding="utf-8")
    assert "exclude test accounts" in report_text.lower()
    assert "requires human review" in report_text.lower()

    replay_result = replay_artifact(result.artifact_id)
    assert replay_result.status == "pass"
