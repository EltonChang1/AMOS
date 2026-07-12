from __future__ import annotations

from amos.memory.models import ClaimRecord, MemoryObject
from amos.memory.store import MemoryStore
from amos.provenance.models import ClaimProvenance


def cite_claims(
    claims: list[ClaimRecord],
    artifact_id: str,
    query_ids: list[str],
    chart_ids: list[str],
    memory_items: list[MemoryObject],
    data_state: dict[str, object],
    execution_state: dict[str, object],
    verification_state: dict[str, object],
    query_kinds: dict[str, str] | None = None,
    store: MemoryStore | None = None,
) -> list[ClaimProvenance]:
    store = store or MemoryStore()
    query_kinds = query_kinds or _infer_query_kinds(query_ids)
    ids_by_kind = _query_ids_by_kind(query_kinds)
    memory_ids_by_type = _memory_ids_by_type(memory_items)

    provenance_records: list[ClaimProvenance] = []
    for claim in claims:
        plan = _support_plan(claim, ids_by_kind, chart_ids, memory_ids_by_type)
        provenance = ClaimProvenance(
            claim_id=claim.claim_id,
            claim_text=claim.claim_text,
            artifact_id=artifact_id,
            support=plan["support"],
            query_ids=plan["query_ids"],
            chart_ids=plan["chart_ids"],
            document_refs=plan["document_refs"],
            memory_object_ids=plan["memory_object_ids"],
            data_state=data_state if plan["data_state"] else {},
            semantic_state={
                "metric_definition_ids": plan["metric_ids"],
                "schema_ids": plan["schema_ids"],
                "stream_state_ids": plan["stream_ids"],
            },
            execution_state=execution_state,
            verification_state=verification_state,
        )
        store.add_claim_provenance(provenance)
        provenance_records.append(provenance)
    return provenance_records


def _support_plan(
    claim: ClaimRecord,
    ids_by_kind: dict[str, list[str]],
    chart_ids: list[str],
    memory_ids_by_type: dict[str, list[str]],
) -> dict[str, list[str] | bool]:
    metric_ids = memory_ids_by_type.get("semantic_definition", [])
    schema_ids = memory_ids_by_type.get("schema", [])
    stream_ids = memory_ids_by_type.get("stream_state", [])
    approved_feedback_ids = memory_ids_by_type.get("approved_feedback", [])
    document_ids = memory_ids_by_type.get("document", [])
    prior_analysis_ids = memory_ids_by_type.get("prior_analysis", [])

    query_ids: list[str]
    claim_chart_ids: list[str] = []
    document_refs: list[str] = []
    memory_object_ids: list[str] = [*metric_ids, *schema_ids, *stream_ids]

    if claim.claim_id.endswith("_rate_increase"):
        query_ids = [*ids_by_kind.get("summary", []), *ids_by_kind.get("timeseries", [])]
        claim_chart_ids = chart_ids[:]
    elif claim.claim_id.endswith("_concentration"):
        query_ids = ids_by_kind.get("concentration", [])[:]
    elif claim.claim_type == "causal":
        query_ids = [*ids_by_kind.get("summary", []), *ids_by_kind.get("concentration", [])]
        document_refs = [*document_ids, *approved_feedback_ids, *prior_analysis_ids]
        memory_object_ids.extend(document_refs)
    elif claim.claim_type == "recommendation":
        query_ids = [
            *ids_by_kind.get("summary", []),
            *ids_by_kind.get("concentration", []),
            *ids_by_kind.get("timeseries", []),
        ]
        claim_chart_ids = chart_ids[:]
        document_refs = [*document_ids, *approved_feedback_ids, *prior_analysis_ids]
        memory_object_ids.extend(document_refs)
    else:
        query_ids = ids_by_kind.get("summary", [])[:]

    support = _unique([*query_ids, *claim_chart_ids, *memory_object_ids])
    return {
        "support": support,
        "query_ids": _unique(query_ids),
        "chart_ids": _unique(claim_chart_ids),
        "document_refs": _unique(document_refs),
        "memory_object_ids": _unique(memory_object_ids),
        "metric_ids": metric_ids,
        "schema_ids": schema_ids,
        "stream_ids": stream_ids,
        "data_state": bool(stream_ids),
    }


def _query_ids_by_kind(query_kinds: dict[str, str]) -> dict[str, list[str]]:
    by_kind: dict[str, list[str]] = {}
    for query_id, kind in query_kinds.items():
        by_kind.setdefault(kind, []).append(query_id)
    return by_kind


def _infer_query_kinds(query_ids: list[str]) -> dict[str, str]:
    inferred: dict[str, str] = {}
    for query_id in query_ids:
        if query_id.endswith("_summary"):
            inferred[query_id] = "summary"
        elif query_id.endswith("_concentration"):
            inferred[query_id] = "concentration"
        elif query_id.endswith("_timeseries"):
            inferred[query_id] = "timeseries"
        else:
            inferred[query_id] = "unknown"
    return inferred


def _memory_ids_by_type(memory_items: list[MemoryObject]) -> dict[str, list[str]]:
    by_type: dict[str, list[str]] = {}
    for item in memory_items:
        by_type.setdefault(item.type, []).append(item.id)
        if item.type == "feedback" and item.authority in {"owner_approved", "reviewer_approved"}:
            by_type.setdefault("approved_feedback", []).append(item.id)
    return by_type


def _unique(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            result.append(value)
    return result
