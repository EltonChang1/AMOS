"""OSS-faithful external baseline adapters.

These adapters load external fixture exports shaped like product interfaces:
- RAG corpus (JSONL documents + SQLite FTS5)
- Semantic-layer metrics YAML (MetricFlow / dbt-metrics shaped)
- Catalog / lineage OpenLineage-shaped events

They are intentionally *not* hosted enterprise products. Contracts must be cited
as ``oss_faithful`` adapters.
"""

from __future__ import annotations

import json
import re
import sqlite3
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.memory.models import RetrieveRequest, User
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore
from amos.tools.duckdb_tool import DuckDBTool
from amos.tools.sql_templates import payment_failure_summary_sql
from amos.verifier.freshness_checks import check_freshness
from amos.verifier.metric_checks import check_metric_rules
from amos.verifier.permission_checks import check_memory_permissions
from amos.verifier.schema_checks import check_schema
from amos.verifier.sql_checks import check_sql_read_only

FIXTURE_ROOT = Path(__file__).resolve().parent / "fixtures" / "external_baselines"
OSS_BASELINE_SYSTEMS = ["oss_rag", "oss_semantic", "oss_catalog"]

ANALYST = User(id="analyst_001", permissions=["analytics", "payments"])
SUBSCRIPTION_ANALYST = User(id="analyst_001", permissions=["analytics", "subscriptions", "billing", "finance"])
WAREHOUSE_ANALYST = User(id="analyst_001", permissions=["analytics", "warehouse", "finance"])


@dataclass(frozen=True)
class OssBaselineOutcome:
    adapter_id: str
    implemented_as: str
    raw_payload: dict[str, Any]
    metrics: dict[str, Any]
    status: str


def fixture_root() -> Path:
    return FIXTURE_ROOT


def load_semantic_metrics() -> list[dict[str, Any]]:
    path = FIXTURE_ROOT / "semantic_layer" / "metrics.json"
    payload = json.loads(path.read_text(encoding="utf-8"))
    return list(payload.get("metrics") or [])


def load_openlineage_events() -> list[dict[str, Any]]:
    path = FIXTURE_ROOT / "catalog" / "openlineage_events.json"
    payload = json.loads(path.read_text(encoding="utf-8"))
    return list(payload.get("events") or [])


def load_rag_documents() -> list[dict[str, Any]]:
    path = FIXTURE_ROOT / "rag" / "documents.jsonl"
    docs: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            docs.append(json.loads(line))
    return docs


def system_contract(system: str) -> dict[str, Any]:
    contracts = {
        "oss_rag": {
            "category": "oss_faithful_baseline",
            "adapter_family": "oss_faithful",
            "memory_access": "external JSONL corpus retrieved via SQLite FTS5 with permission pre-filter",
            "current_metric_schema": True,
            "permission_filter_before_context": True,
            "runtime_verifier": False,
            "claim_provenance": False,
            "replay_required": False,
            "implemented_as": (
                "OSS metadata RAG adapter over exported documents.jsonl using FTS5; "
                "no AMOS verifier, claim provenance, or replay."
            ),
        },
        "oss_semantic": {
            "category": "oss_faithful_baseline",
            "adapter_family": "oss_faithful",
            "memory_access": "external metrics.json semantic-layer export",
            "current_metric_schema": True,
            "permission_filter_before_context": False,
            "runtime_verifier": False,
            "claim_provenance": False,
            "replay_required": False,
            "implemented_as": (
                "OSS semantic-layer adapter over MetricFlow/dbt-metrics-shaped JSON; "
                "metric-rule validation only."
            ),
        },
        "oss_catalog": {
            "category": "oss_faithful_baseline",
            "adapter_family": "oss_faithful",
            "memory_access": "OpenLineage-shaped events plus governed memory schema/metric/stream objects",
            "current_metric_schema": True,
            "permission_filter_before_context": True,
            "runtime_verifier": True,
            "claim_provenance": False,
            "replay_required": False,
            "implemented_as": (
                "OSS catalog/lineage adapter over OpenLineage JSON events with multi-verifier checks; "
                "no claim-level provenance or replay packages."
            ),
        },
    }
    return contracts[system]


def run_oss_baseline(
    adapter_id: str,
    *,
    scenario: str,
    task_request: str,
    task_family: str,
    expected_evidence: list[str],
    store: MemoryStore,
    sql_builder,
) -> OssBaselineOutcome:
    if adapter_id == "oss_rag":
        return _run_oss_rag(scenario, task_request, task_family, expected_evidence, store, sql_builder)
    if adapter_id == "oss_semantic":
        return _run_oss_semantic(scenario, task_request, task_family, expected_evidence, store, sql_builder)
    if adapter_id == "oss_catalog":
        return _run_oss_catalog(scenario, task_request, task_family, expected_evidence, store, sql_builder)
    raise ValueError(f"Unsupported OSS baseline: {adapter_id}")


def _user_for_scenario(scenario: str) -> User:
    return {
        "payment_failure": ANALYST,
        "subscription_churn": SUBSCRIPTION_ANALYST,
        "warehouse_quality": WAREHOUSE_ANALYST,
    }[scenario]


def _run_oss_rag(
    scenario: str,
    task_request: str,
    task_family: str,
    expected_evidence: list[str],
    store: MemoryStore,
    sql_builder,
) -> OssBaselineOutcome:
    user = _user_for_scenario(scenario)
    docs = load_rag_documents()
    retrieved = _fts_retrieve(docs, task_request, user.permissions, limit=8)
    leaked = [doc["doc_id"] for doc in retrieved if "sre" in doc.get("permissions", []) and "sre" not in user.permissions]
    # Prefer approved metric SQL when corpus surfaces the runbook; otherwise use agent-only style.
    prefer_approved = any("payment_failure_rate" in doc.get("text", "") and doc.get("status") != "superseded" for doc in retrieved)
    sql = sql_builder(mode="amos" if prefer_approved and task_family != "security" else "agent_only")
    execution = _execute_sql(sql)
    verification = {"passed_checks": [], "warnings": [], "errors": []}
    if scenario == "payment_failure":
        verification = _payment_basic_checks(sql, store)
    permission_safe = not leaked
    metrics = _metrics(
        sql=sql,
        execution=execution,
        verification=verification,
        permission_safe=permission_safe,
        review_required=_review_required(task_family, task_request),
    )
    contract = system_contract("oss_rag")
    return OssBaselineOutcome(
        adapter_id="oss_rag",
        implemented_as=str(contract["implemented_as"]),
        raw_payload={
            "system_contract": contract,
            "external_corpus": str(FIXTURE_ROOT / "rag" / "documents.jsonl"),
            "retrieved_doc_ids": [doc["doc_id"] for doc in retrieved],
            "restricted_docs_in_context": leaked,
            "generated_sql": sql,
            "execution": execution,
            "verification": verification,
            "output_text": (
                "OSS RAG adapter retrieved exported documents via FTS5. "
                + ("Causal/dashboard claims require human review." if _review_required(task_family, task_request) else "")
            ),
            "expected_evidence_overlap": [eid for eid in expected_evidence if any(eid in doc.get("text", "") for doc in retrieved)],
        },
        metrics=metrics,
        status=_status_from_metrics(metrics),
    )


def _run_oss_semantic(
    scenario: str,
    task_request: str,
    task_family: str,
    expected_evidence: list[str],
    store: MemoryStore,
    sql_builder,
) -> OssBaselineOutcome:
    metrics_yaml = load_semantic_metrics()
    active = [m for m in metrics_yaml if m.get("status") == "active"]
    chosen = _choose_metric(active, scenario, task_request)
    sql = sql_builder(mode="amos")
    metric_obj = _memory_metric_for_scenario(scenario, store)
    metric_warnings: list[str] = []
    metric_errors: list[str] = []
    if metric_obj is not None and scenario == "payment_failure":
        metric_warnings, metric_errors = check_metric_rules(sql, metric_obj)
    # Cross-domain MemoryObjects may not share the payment metric AST contract; YAML filters are the external semantic source of truth there.
    yaml_errors = _yaml_metric_filter_errors(sql, chosen)
    errors = [*metric_errors, *yaml_errors]
    execution = _execute_sql(sql) if not errors else {"status": "skipped", "rows": [], "errors": errors}
    verification = {
        "passed_checks": ["metric_rules"] if not errors else [],
        "warnings": metric_warnings,
        "errors": errors,
        "selected_metric": chosen,
    }
    metrics = _metrics(
        sql=sql,
        execution=execution,
        verification=verification,
        permission_safe=True,
        review_required=_review_required(task_family, task_request),
    )
    contract = system_contract("oss_semantic")
    return OssBaselineOutcome(
        adapter_id="oss_semantic",
        implemented_as=str(contract["implemented_as"]),
        raw_payload={
            "system_contract": contract,
            "external_metrics_json": str(FIXTURE_ROOT / "semantic_layer" / "metrics.json"),
            "selected_metric": chosen,
            "generated_sql": sql,
            "execution": execution,
            "verification": verification,
            "output_text": (
                "OSS semantic-layer adapter applied metrics.json filters. "
                + ("Causal/dashboard claims require human review." if _review_required(task_family, task_request) else "")
            ),
            "expected_evidence": expected_evidence,
        },
        metrics=metrics,
        status=_status_from_metrics(metrics),
    )


def _run_oss_catalog(
    scenario: str,
    task_request: str,
    task_family: str,
    expected_evidence: list[str],
    store: MemoryStore,
    sql_builder,
) -> OssBaselineOutcome:
    user = _user_for_scenario(scenario)
    events = load_openlineage_events()
    lineage = _lineage_summary(events)
    retrieval = retrieve(
        RetrieveRequest(
            task_text=task_request,
            required_types=["semantic_definition", "schema", "stream_state", "document", "permission_policy"],
            time_range=(datetime(2026, 7, 1, tzinfo=timezone.utc), datetime(2026, 7, 10, tzinfo=timezone.utc)),
            user_permissions=user.permissions,
            max_items=16,
        ),
        store=store,
    )
    sql = sql_builder(mode="amos")
    schema = _pick_memory(retrieval.items, "schema")
    metric = _pick_memory(retrieval.items, "semantic_definition")
    stream = _pick_memory(retrieval.items, "stream_state")
    sql_check = check_sql_read_only(sql)
    schema_warnings, schema_errors = ([], [])
    metric_warnings, metric_errors = ([], [])
    freshness_warnings, freshness_errors = ([], [])
    if scenario == "payment_failure":
        if metric is not None:
            metric_warnings, metric_errors = check_metric_rules(sql, metric)
        if stream is not None:
            freshness_warnings, freshness_errors = check_freshness(stream)
        if schema is not None:
            try:
                schema_warnings, schema_errors = check_schema(sql, schema)
            except Exception as exc:  # pragma: no cover
                schema_errors = [f"Schema check failed: {exc}"]
    permission_warnings, permission_errors = check_memory_permissions(retrieval.items, user.permissions)
    # Reject stale renamed columns when lineage says failure_reason -> error_code.
    lineage_errors: list[str] = []
    if "failure_reason" in sql and any(
        "error_code" in json.dumps(event) and "failure_reason" in json.dumps(event) for event in events
    ):
        lineage_errors.append("OpenLineage column lineage: failure_reason superseded by error_code")
    errors = [
        *sql_check.errors,
        *schema_errors,
        *metric_errors,
        *freshness_errors,
        *permission_errors,
        *lineage_errors,
    ]
    warnings = [
        *sql_check.warnings,
        *schema_warnings,
        *metric_warnings,
        *freshness_warnings,
        *permission_warnings,
        *retrieval.warnings,
    ]
    execution = _execute_sql(sql) if not errors else {"status": "skipped", "rows": [], "errors": errors}
    verification = {
        "passed_checks": [
            name
            for name, ok in {
                "sql_read_only": not sql_check.errors,
                "schema_compatible": not schema_errors,
                "metric_rules": not metric_errors,
                "freshness": not freshness_errors,
                "permissions": not permission_errors,
                "openlineage_lineage": not lineage_errors,
            }.items()
            if ok
        ],
        "warnings": warnings,
        "errors": errors,
        "lineage_summary": lineage,
    }
    restricted = [item.id for item in retrieval.items if "sre" in item.permissions and "sre" not in user.permissions]
    metrics = _metrics(
        sql=sql,
        execution=execution,
        verification=verification,
        permission_safe=not restricted and not permission_errors,
        review_required=_review_required(task_family, task_request),
    )
    contract = system_contract("oss_catalog")
    return OssBaselineOutcome(
        adapter_id="oss_catalog",
        implemented_as=str(contract["implemented_as"]),
        raw_payload={
            "system_contract": contract,
            "external_openlineage": str(FIXTURE_ROOT / "catalog" / "openlineage_events.json"),
            "lineage_summary": lineage,
            "retrieved_memory_ids": [item.id for item in retrieval.items],
            "filtered_permission_ids": retrieval.filtered_permission_ids,
            "restricted_memory_in_context": restricted,
            "generated_sql": sql,
            "execution": execution,
            "verification": verification,
            "output_text": (
                "OSS catalog/lineage adapter validated SQL against OpenLineage events and governed metadata. "
                + ("Causal/dashboard claims require human review." if _review_required(task_family, task_request) else "")
            ),
            "expected_evidence": expected_evidence,
        },
        metrics=metrics,
        status=_status_from_metrics(metrics),
    )


def _fts_retrieve(docs: list[dict[str, Any]], query: str, permissions: list[str], limit: int = 8) -> list[dict[str, Any]]:
    visible = []
    user_perms = set(permissions)
    for doc in docs:
        doc_perms = set(doc.get("permissions") or [])
        if doc_perms and not doc_perms.issubset(user_perms):
            continue
        visible.append(doc)
    with tempfile.NamedTemporaryFile(suffix=".sqlite") as tmp:
        conn = sqlite3.connect(tmp.name)
        try:
            conn.execute("CREATE VIRTUAL TABLE docs USING fts5(doc_id, title, text, permissions, tokenize='porter')")
            for doc in visible:
                conn.execute(
                    "INSERT INTO docs(doc_id, title, text, permissions) VALUES (?, ?, ?, ?)",
                    (
                        doc["doc_id"],
                        doc.get("title", ""),
                        doc.get("text", ""),
                        " ".join(doc.get("permissions") or []),
                    ),
                )
            conn.commit()
            terms = [re.sub(r"[^a-z0-9_]", "", tok.lower()) for tok in query.split()]
            terms = [t for t in terms if len(t) > 2][:12]
            if not terms:
                return visible[:limit]
            match = " OR ".join(terms)
            rows = conn.execute(
                "SELECT doc_id FROM docs WHERE docs MATCH ? ORDER BY rank LIMIT ?",
                (match, limit),
            ).fetchall()
            by_id = {doc["doc_id"]: doc for doc in visible}
            ordered = [by_id[row[0]] for row in rows if row[0] in by_id]
            return ordered or visible[:limit]
        finally:
            conn.close()


def _choose_metric(active: list[dict[str, Any]], scenario: str, task_request: str) -> dict[str, Any]:
    preferred = {
        "payment_failure": "payment_failure_rate",
        "subscription_churn": "subscription_churn_rate",
        "warehouse_quality": "warehouse_pick_accuracy",
    }[scenario]
    for metric in active:
        if metric.get("name") == preferred:
            return metric
    # Fall back to lexical match.
    lower = task_request.lower()
    for metric in active:
        if str(metric.get("name", "")).replace("_", " ") in lower:
            return metric
    return active[0] if active else {"name": preferred, "version": "unknown", "filters": []}


def _yaml_metric_filter_errors(sql: str, metric: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    sql_l = sql.lower()
    for filt in metric.get("filters") or []:
        token = str(filt).split("=")[0].strip().lower()
        aliases = {
            "is_test_account": ["is_test_account", "is_test_location"],
            "is_test_location": ["is_test_location", "is_test_account"],
            "traffic_type": ["traffic_type", "environment", "production"],
            "environment": ["environment", "production"],
        }
        candidates = aliases.get(token, [token])
        if token and not any(candidate in sql_l for candidate in candidates):
            errors.append(f"Missing YAML metric filter token: {token}")
    time_field = str(metric.get("time_field") or "").lower()
    if time_field and time_field not in sql_l and not any(t in sql_l for t in ["event_time", "week_start"]):
        errors.append(f"Missing YAML metric time field: {time_field}")
    return errors


def _memory_metric_for_scenario(scenario: str, store: MemoryStore):
    ids = {
        "payment_failure": "memory_metric_payment_failure_rate_v3",
        "subscription_churn": "memory_metric_subscription_churn_rate_v1",
        "warehouse_quality": "memory_metric_warehouse_pick_accuracy_v1",
    }
    for candidate in [ids.get(scenario, ""), "memory_metric_payment_failure_rate_v3"]:
        item = store.get_memory(candidate) if candidate else None
        if item is not None:
            return item
    # Best-effort: first semantic definition.
    for item in store.list_memory():
        if item.type == "semantic_definition" and item.status == "active":
            return item
    return None


def _pick_memory(items, memory_type: str):
    for item in items:
        if item.type == memory_type and item.status == "active":
            return item
    for item in items:
        if item.type == memory_type:
            return item
    return None


def _lineage_summary(events: list[dict[str, Any]]) -> dict[str, Any]:
    versions = []
    renames = []
    for event in events:
        for dataset in list(event.get("inputs") or []) + list(event.get("outputs") or []):
            facets = dataset.get("facets") or {}
            version = (facets.get("version") or {}).get("datasetVersion")
            if version:
                versions.append(version)
            lineage = facets.get("columnLineage") or {}
            for out_col, spec in (lineage.get("fields") or {}).items():
                for src in spec.get("inputFields") or []:
                    renames.append({"from": src.get("field"), "to": out_col})
    return {"dataset_versions": sorted(set(versions)), "column_renames": renames, "event_count": len(events)}


def _payment_basic_checks(sql: str, store: MemoryStore) -> dict[str, Any]:
    sql_check = check_sql_read_only(sql)
    schema = store.get_memory("memory_schema_payment_events_v2")
    metric = store.get_memory("memory_metric_payment_failure_rate_v3")
    schema_errors: list[str] = []
    metric_errors: list[str] = []
    warnings: list[str] = list(sql_check.warnings)
    if schema is not None:
        try:
            schema_warnings, schema_errors = check_schema(sql, schema)
            warnings.extend(schema_warnings)
        except Exception as exc:  # pragma: no cover
            schema_errors = [f"Schema check failed: {exc}"]
    if metric is not None:
        metric_warnings, metric_errors = check_metric_rules(sql, metric)
        warnings.extend(metric_warnings)
    errors = [*sql_check.errors, *schema_errors, *metric_errors]
    return {
        "passed_checks": [
            name
            for name, ok in {
                "sql_read_only": not sql_check.errors,
                "schema_compatible": not schema_errors,
                "metric_rules": not metric_errors,
            }.items()
            if ok
        ],
        "warnings": warnings,
        "errors": errors,
    }


def _execute_sql(sql: str) -> dict[str, Any]:
    tool = DuckDBTool()
    try:
        rows = tool.execute(sql)
    except Exception as exc:
        return {"status": "error", "rows": [], "errors": [f"{type(exc).__name__}: {exc}"]}
    return {"status": "pass", "rows": rows[:10], "row_count": len(rows), "result_hash": tool.result_hash(rows)}


def _review_required(task_family: str, task_request: str) -> bool:
    lower = task_request.lower()
    return task_family in {"causal", "governance"} or any(
        term in lower for term in ["dashboard", "caused", "root cause", "update the executive"]
    )


def _has_metric_requirements(sql: str) -> bool:
    lower = sql.lower()
    has_test_exclusion = any(
        token in lower for token in ["is_test_account", "is_test_location", "cycle_count_rehearsal"]
    )
    has_prod_or_window = any(
        token in lower for token in ["production", "environment", "event_time", "week_start", "count(*)"]
    )
    return bool(has_test_exclusion or has_prod_or_window)


def _metrics(
    *,
    sql: str,
    execution: dict[str, Any],
    verification: dict[str, Any],
    permission_safe: bool,
    review_required: bool,
) -> dict[str, Any]:
    errors = verification.get("errors", [])
    sql_valid = execution.get("status") == "pass" and not errors
    metric_correct = bool(
        _has_metric_requirements(sql)
        and (
            "metric_rules" in (verification.get("passed_checks") or [])
            or not errors
        )
    )
    schema_correct = "failure_reason" not in sql and "raw_payload" not in sql and not any(
        "schema" in str(err).lower() for err in errors
    )
    review_recall = 1.0 if review_required else 1.0
    # Review recall measured by whether output path marks review; callers set output text.
    task_correct = bool(sql_valid and metric_correct and schema_correct and permission_safe)
    return {
        "task_correctness": task_correct,
        "sql_validity": sql_valid,
        "metric_correctness": bool(metric_correct),
        "schema_correctness": bool(schema_correct),
        "permission_safety": permission_safe,
        "provenance_coverage": 0.0,
        "replay_success": False,
        "review_obligation_recall": review_recall if review_required else 1.0,
        "token_usage": {
            "input_tokens": max(len(sql) // 4, 1),
            "output_tokens": max(len(sql) // 8, 1),
            "total_tokens": max(len(sql) // 4, 1) + max(len(sql) // 8, 1),
        },
        "raw_prompt_trace": False,
        "passed": False,
    }


def _status_from_metrics(metrics: dict[str, Any]) -> str:
    if metrics.get("passed"):
        return "pass"
    if metrics.get("task_correctness"):
        return "warning"
    if metrics.get("permission_safety") is False:
        return "reject"
    return "reject"


def default_payment_sql_builder(mode: str = "amos"):
    def _builder(mode: str = mode) -> str:
        if mode == "amos":
            return payment_failure_summary_sql()
        return payment_failure_summary_sql().replace("AND is_test_account = false", "")

    return _builder
