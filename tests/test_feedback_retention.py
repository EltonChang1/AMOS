from __future__ import annotations

from datetime import datetime

from amos.agent.controller import write_feedback
from amos.memory.models import RetrieveRequest
from amos.memory.retrieval import retrieve
from amos.tools.sql_templates import PAYMENT_WINDOW_END, PAYMENT_WINDOW_START


def test_reviewer_feedback_is_retrieved_for_later_payment_task(seeded: None) -> None:
    item = write_feedback(
        artifact_id="report_test",
        reviewer_role="payments_analytics_lead",
        feedback="Do not attribute the whole spike to the deployment; processor-specific evidence is required.",
        effective_start=datetime.fromisoformat(PAYMENT_WINDOW_START),
    )

    result = retrieve(
        RetrieveRequest(
            task_text="Why did payment failure rate increase over the last six hours?",
            required_types=["feedback"],
            time_range=(datetime.fromisoformat(PAYMENT_WINDOW_START), datetime.fromisoformat(PAYMENT_WINDOW_END)),
            user_permissions=["analytics", "payments"],
        )
    )

    assert item.id in {memory.id for memory in result.items}
