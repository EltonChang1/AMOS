from __future__ import annotations

from pydantic import BaseModel

from amos.memory.store import MemoryStore
from amos.tools.chart_tool import create_failure_rate_chart
from amos.tools.duckdb_tool import DuckDBTool


class ReplayResult(BaseModel):
    artifact_id: str
    status: str
    warnings: list[str] = []
    errors: list[str] = []


def replay_artifact(artifact_id: str, store: MemoryStore | None = None) -> ReplayResult:
    store = store or MemoryStore()
    package = store.get_replay_package(artifact_id)
    if package is None:
        return ReplayResult(artifact_id=artifact_id, status="fail", errors=["No replay package found."])

    tool = DuckDBTool()
    warnings: list[str] = []
    errors: list[str] = []
    for query_id in package.query_ids:
        query_info = package.task_plan["queries"][query_id]
        rows = tool.execute(query_info["sql"])
        actual_hash = tool.result_hash(rows)
        if actual_hash != query_info["result_hash"]:
            errors.append(f"Query {query_id} result hash changed.")
        if query_info["kind"] == "timeseries":
            create_failure_rate_chart(rows, package.chart_ids[0])

    status = "fail" if errors else ("warning" if warnings else "pass")
    store.log("artifact.replay", "agent", {"artifact_id": artifact_id}, {"status": status}, status)
    return ReplayResult(artifact_id=artifact_id, status=status, warnings=warnings, errors=errors)
