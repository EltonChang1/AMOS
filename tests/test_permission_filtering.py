from __future__ import annotations

from datetime import datetime

from amos.memory.models import RetrieveRequest
from amos.memory.retrieval import retrieve
from amos.tools.sql_templates import PAYMENT_WINDOW_END, PAYMENT_WINDOW_START


def test_restricted_incident_is_filtered_before_context(seeded: None) -> None:
    result = retrieve(
        RetrieveRequest(
            task_text="payment processor retry amplification incident",
            required_types=["prior_analysis"],
            time_range=(datetime.fromisoformat(PAYMENT_WINDOW_START), datetime.fromisoformat(PAYMENT_WINDOW_END)),
            user_permissions=["analytics", "payments"],
        )
    )
    assert not result.items
    assert "memory_prior_processor_retry_amplification" in result.filtered_permission_ids
