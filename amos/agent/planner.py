from __future__ import annotations

from pydantic import BaseModel

from amos.agent.task_parser import ParsedTask


class AnalysisPlan(BaseModel):
    plan_id: str
    required_memory_types: list[str]
    query_kinds: list[str]
    chart_kinds: list[str]
    provenance_level: int


def plan_task(parsed: ParsedTask, provenance_level: int) -> AnalysisPlan:
    return AnalysisPlan(
        plan_id=f"plan_{parsed.task_id}",
        required_memory_types=[
            "semantic_definition",
            "schema",
            "stream_state",
            "prior_analysis",
            "document",
            "feedback",
            "permission_policy",
        ],
        query_kinds=["summary", "concentration", "timeseries"],
        chart_kinds=["failure_rate_timeseries"],
        provenance_level=provenance_level,
    )
