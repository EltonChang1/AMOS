from __future__ import annotations

from amos.memory.models import RunTaskResult


def summarize_amos_result(result: RunTaskResult, replay_status: str, feedback_retained: bool, permission_safe: bool) -> dict[str, object]:
    return {
        "task_correctness": result.verification_status in {"pass", "warning"},
        "temporal_correctness": any("stream_payment_events" in memory_id for memory_id in result.used_memory_ids),
        "metric_correctness": "memory_metric_payment_failure_rate_v3" in result.used_memory_ids,
        "schema_correctness": "memory_schema_payment_events_v2" in result.used_memory_ids,
        "provenance_coverage": result.provenance_coverage,
        "replay_success": replay_status == "pass",
        "feedback_retention": feedback_retained,
        "permission_safety": permission_safe,
    }
