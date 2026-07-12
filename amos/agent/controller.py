from __future__ import annotations

import argparse
import json
import time
import uuid
from datetime import datetime, timezone

from amos.agent.planner import plan_task
from amos.agent.report_generator import generate_report
from amos.agent.task_parser import parse_task
from amos.config import settings
from amos.memory.models import ArtifactRecord, MemoryObject, ReplayPackage, RetrieveRequest, RunTaskResult, User
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore
from amos.provenance.claim_extractor import build_claims
from amos.provenance.recorder import cite_claims
from amos.tools.artifact_store import write_json_artifact, write_text_artifact
from amos.tools.chart_tool import create_failure_rate_chart
from amos.tools.duckdb_tool import DuckDBTool
from amos.tools.sql_templates import (
    payment_failure_concentration_sql,
    payment_failure_summary_sql,
    payment_failure_timeseries_sql,
)
from amos.verifier.verifier import verify_provenance, verify_sql


def run_amos_task(request: str, user: User, provenance_level: int = 3) -> RunTaskResult:
    settings.ensure_dirs()
    store = MemoryStore()
    store.init_schema()
    task_id = f"task_{uuid.uuid4().hex[:12]}"
    artifact_id = f"report_{uuid.uuid4().hex[:12]}"
    chart_id = f"chart_{uuid.uuid4().hex[:12]}"
    replay_package_id = f"replay_{uuid.uuid4().hex[:12]}"
    started = time.perf_counter()

    parsed = parse_task(request, task_id)
    plan = plan_task(parsed, provenance_level)
    retrieval = retrieve(
        RetrieveRequest(
            task_text=request,
            required_types=plan.required_memory_types,
            time_range=parsed.time_range,
            user_permissions=user.permissions,
            max_items=12,
        ),
        store=store,
    )
    memory_items = retrieval.items
    metric = _required(memory_items, "semantic_definition")
    schema = _required(memory_items, "schema")
    stream_state = _required(memory_items, "stream_state")

    sqls = {
        f"query_{artifact_id}_summary": ("summary", payment_failure_summary_sql()),
        f"query_{artifact_id}_concentration": ("concentration", payment_failure_concentration_sql()),
        f"query_{artifact_id}_timeseries": ("timeseries", payment_failure_timeseries_sql()),
    }

    sql_verifications = [
        verify_sql(sql, schema, metric, stream_state, memory_items, user.permissions) for _, sql in sqls.values()
    ]
    failed = [result for result in sql_verifications if result.status == "fail"]
    if failed:
        errors = [error for result in failed for error in result.errors]
        raise RuntimeError(f"AMOS SQL verification failed: {errors}")

    duckdb_tool = DuckDBTool()
    query_results: dict[str, dict[str, object]] = {}
    for query_id, (kind, sql) in sqls.items():
        rows = duckdb_tool.execute(sql)
        result_hash = duckdb_tool.result_hash(rows)
        query_path = write_text_artifact(settings.queries_dir, query_id, "sql", sql)
        query_results[query_id] = {
            "kind": kind,
            "sql": sql,
            "path": str(query_path),
            "rows": rows,
            "result_hash": result_hash,
        }

    summary_rows = query_results[f"query_{artifact_id}_summary"]["rows"]
    summary_by_period = {row["period"]: row for row in summary_rows}  # type: ignore[index]
    previous_rate = float(summary_by_period["previous"]["failure_rate"])
    current_rate = float(summary_by_period["current"]["failure_rate"])
    concentration_rows = query_results[f"query_{artifact_id}_concentration"]["rows"]
    top_segment = concentration_rows[0]  # type: ignore[index]
    timeseries_rows = query_results[f"query_{artifact_id}_timeseries"]["rows"]
    chart_path = create_failure_rate_chart(timeseries_rows, chart_id)  # type: ignore[arg-type]

    dashboard_recommendation = (
        "Update the executive dashboard with a warning annotation for the spike window and keep the cause marked "
        "pending review."
    )
    claims = build_claims(
        artifact_id=artifact_id,
        previous_rate=previous_rate,
        current_rate=current_rate,
        top_processor=str(top_segment["processor"]),
        top_network=str(top_segment["card_network"]),
        dashboard_recommendation=dashboard_recommendation,
    )
    for claim in claims:
        store.add_claim(claim)

    sql_warnings = _unique([warning for result in sql_verifications for warning in result.warnings])
    data_state = stream_state.content
    execution_state = {
        "engine": "duckdb",
        "queries": {query_id: {"hash": info["result_hash"], "path": info["path"]} for query_id, info in query_results.items()},
        "latency_seconds": round(time.perf_counter() - started, 4),
    }
    verification_state = {
        "sql_statuses": [result.status for result in sql_verifications],
        "passed_checks": sorted({check for result in sql_verifications for check in result.passed_checks}),
        "warnings": sql_warnings,
    }

    provenance_records = cite_claims(
        claims=claims,
        artifact_id=artifact_id,
        query_ids=list(sqls.keys()),
        chart_ids=[chart_id],
        memory_items=memory_items,
        data_state=data_state,
        execution_state=execution_state,
        verification_state=verification_state,
        query_kinds={query_id: str(info["kind"]) for query_id, info in query_results.items()},
        store=store,
    )
    provenance_verification = verify_provenance(claims, provenance_records, provenance_level)
    all_warnings = _unique([*retrieval.warnings, *sql_warnings, *provenance_verification.warnings])
    verification_status = _combine_status([*(result.status for result in sql_verifications), provenance_verification.status])

    package = ReplayPackage(
        replay_package_id=replay_package_id,
        artifact_id=artifact_id,
        user_request=request,
        task_plan={
            **plan.model_dump(mode="json"),
            "queries": {
                query_id: {
                    "kind": info["kind"],
                    "sql": info["sql"],
                    "path": info["path"],
                    "result_hash": info["result_hash"],
                }
                for query_id, info in query_results.items()
            },
        },
        query_ids=list(sqls.keys()),
        chart_ids=[chart_id],
        memory_snapshot_ids=[item.id for item in memory_items],
        schema_versions=[schema.id],
        semantic_definition_versions=[metric.id],
        stream_or_snapshot_state=stream_state.content,
        tool_versions={"duckdb": "local", "amos": "0.1.0"},
        verification_report_id=f"verification_{artifact_id}",
    )
    store.add_replay_package(package)
    write_json_artifact(settings.replay_dir, replay_package_id, package.model_dump(mode="json"))

    report_text = generate_report(
        artifact_id=artifact_id,
        previous_rate=previous_rate,
        current_rate=current_rate,
        top_segment=top_segment,  # type: ignore[arg-type]
        chart_path=chart_path,
        claims=claims,
        memory_items=memory_items,
        verification_status=verification_status,
        warnings=all_warnings,
        replay_package_id=replay_package_id,
    )
    report_path = write_text_artifact(settings.reports_dir, artifact_id, "md", report_text)
    provenance_path = write_json_artifact(
        settings.provenance_dir,
        f"provenance_{artifact_id}",
        {"claims": [record.model_dump(mode="json") for record in provenance_records]},
    )

    artifact = ArtifactRecord(
        artifact_id=artifact_id,
        artifact_type="report",
        path=str(report_path),
        user_request=request,
        task_plan_id=plan.plan_id,
        created_by=user.id,
        provenance_ids=[record.claim_id for record in provenance_records],
        replay_package_id=replay_package_id,
    )
    store.add_artifact(artifact)
    store.update_artifact_provenance(artifact_id, artifact.provenance_ids, replay_package_id)
    store.log(
        "task.run",
        user.id,
        {"request": request, "permissions": user.permissions},
        {"artifact_id": artifact_id, "provenance": str(provenance_path), "status": verification_status},
        verification_status,
        task_id=task_id,
    )

    return RunTaskResult(
        task_id=task_id,
        artifact_id=artifact_id,
        report_path=str(report_path),
        chart_paths=[str(chart_path)],
        verification_status=verification_status,
        warnings=all_warnings,
        provenance_ids=[record.claim_id for record in provenance_records],
        replay_package_id=replay_package_id,
        used_memory_ids=[item.id for item in memory_items],
        provenance_coverage=provenance_verification.provenance_coverage,
    )


def write_feedback(
    artifact_id: str,
    reviewer_role: str,
    feedback: str,
    authority: str = "reviewer_approved",
    effective_start: datetime | None = None,
) -> MemoryObject:
    if authority == "owner_approved":
        raise ValueError("Feedback endpoint cannot create owner-approved memory.")
    store = MemoryStore()
    item = MemoryObject(
        id=f"memory_feedback_{uuid.uuid4().hex[:12]}",
        type="feedback",
        summary=f"Reusable reviewer feedback for payment failure investigation {artifact_id}: {feedback}",
        content={
            "artifact_id": artifact_id,
            "feedback": feedback,
            "reviewer_role": reviewer_role,
            "applies_to": "payment failure rate investigation",
        },
        source="reviewer",
        authority=authority,  # type: ignore[arg-type]
        effective_start=effective_start or datetime.now(timezone.utc),
        permissions=["analytics", "payments"],
        version=datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S"),
        status="active",
        provenance_ref=artifact_id,
    )
    store.upsert_memory(item)
    return item


def _required(items: list[MemoryObject], memory_type: str) -> MemoryObject:
    matches = [item for item in items if item.type == memory_type]
    if not matches:
        raise RuntimeError(f"Required AMOS memory type missing: {memory_type}")
    return matches[0]


def _combine_status(statuses: list[str]) -> str:
    if "fail" in statuses:
        return "fail"
    if "warning" in statuses:
        return "warning"
    return "pass"


def _unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            result.append(value)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description="Run the AMOS payment-failure prototype analysis.")
    parser.add_argument("--request", required=True)
    parser.add_argument("--user", default="analyst_001")
    parser.add_argument("--permissions", default="analytics,payments")
    parser.add_argument("--provenance-level", default=3, type=int)
    args = parser.parse_args()
    user = User(id=args.user, permissions=[permission.strip() for permission in args.permissions.split(",") if permission.strip()])
    result = run_amos_task(args.request, user, args.provenance_level)
    print(json.dumps(result.model_dump(mode="json"), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
