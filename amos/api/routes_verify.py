from __future__ import annotations

from fastapi import APIRouter, Depends

from amos.agent.planner import plan_task
from amos.agent.task_parser import parse_task
from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import (
    VerifyArtifactRequest,
    VerifyResponse,
    VerifySqlRequest,
    api_status,
    new_run_id,
)
from amos.memory.models import MemoryObject, RetrieveRequest
from amos.memory.permissions import has_required_permissions
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore
from amos.verifier.verifier import verify_provenance, verify_sql


router = APIRouter(prefix="/verify", tags=["verify"])


@router.post("/sql", response_model=VerifyResponse)
def verify_sql_endpoint(
    payload: VerifySqlRequest,
    identity: Identity = Depends(get_identity),
) -> VerifyResponse:
    run_id = new_run_id()
    store = MemoryStore()
    memory_items, filtered_permission_ids, retrieval_warnings = _resolve_memory(payload, identity, run_id, store)
    errors: list[str] = []
    required = {
        "schema": _first_memory(memory_items, "schema"),
        "semantic_definition": _first_memory(memory_items, "semantic_definition"),
        "stream_state": _first_memory(memory_items, "stream_state"),
    }
    for memory_type, item in required.items():
        if item is None:
            errors.append(f"Required AMOS memory type missing: {memory_type}")
    if errors:
        store.log(
            "api.verify.sql",
            identity.user_id,
            {"run_id": run_id, "sql": payload.sql},
            {"errors": errors, "filtered_permission_ids": filtered_permission_ids},
            "reject",
        )
        return VerifyResponse(
            **api_context(identity, run_id),
            status="reject",
            warnings=retrieval_warnings,
            errors=errors,
            used_memory_ids=[item.id for item in memory_items],
            filtered_permission_ids=filtered_permission_ids,
        )

    verification = verify_sql(
        payload.sql,
        required["schema"],  # type: ignore[arg-type]
        required["semantic_definition"],  # type: ignore[arg-type]
        required["stream_state"],  # type: ignore[arg-type]
        memory_items,
        identity.permissions,
    )
    status = api_status(verification.status)
    if retrieval_warnings and status == "pass":
        status = "warning"
    store.log(
        "api.verify.sql",
        identity.user_id,
        {"run_id": run_id, "sql": payload.sql, "used_memory_ids": [item.id for item in memory_items]},
        {"status": status, "warnings": [*retrieval_warnings, *verification.warnings], "errors": verification.errors},
        status,
    )
    return VerifyResponse(
        **api_context(identity, run_id),
        status=status,
        verification=verification,
        passed_checks=verification.passed_checks,
        warnings=[*retrieval_warnings, *verification.warnings],
        errors=verification.errors,
        used_memory_ids=[item.id for item in memory_items],
        filtered_permission_ids=filtered_permission_ids,
        provenance_coverage=verification.provenance_coverage,
    )


@router.post("/artifact", response_model=VerifyResponse)
def verify_artifact_endpoint(
    payload: VerifyArtifactRequest,
    identity: Identity = Depends(get_identity),
) -> VerifyResponse:
    run_id = new_run_id()
    store = MemoryStore()
    artifact = store.get_artifact(payload.artifact_id)
    if artifact is None:
        errors = ["Artifact not found."]
        store.log("api.verify.artifact", identity.user_id, payload.model_dump(), {"errors": errors}, "reject")
        return VerifyResponse(**api_context(identity, run_id), status="reject", errors=errors)

    claims = store.list_claims(payload.artifact_id)
    provenance = store.list_claim_provenance(payload.artifact_id)
    if not claims or not provenance:
        warnings = ["Artifact is missing claims or claim-level provenance."]
        store.log("api.verify.artifact", identity.user_id, payload.model_dump(), {"warnings": warnings}, "needs_review")
        return VerifyResponse(
            **api_context(identity, run_id),
            status="needs_review",
            warnings=warnings,
            used_memory_ids=[],
        )

    verification = verify_provenance(claims, provenance, payload.provenance_level)
    used_memory_ids = sorted({memory_id for record in provenance for memory_id in record.memory_object_ids})
    status = api_status(verification.status)
    store.log(
        "api.verify.artifact",
        identity.user_id,
        {"run_id": run_id, "artifact_id": payload.artifact_id},
        {"status": status, "provenance_coverage": verification.provenance_coverage},
        status,
    )
    return VerifyResponse(
        **api_context(identity, run_id),
        status=status,
        verification=verification,
        passed_checks=verification.passed_checks,
        warnings=verification.warnings,
        errors=verification.errors,
        used_memory_ids=used_memory_ids,
        provenance_coverage=verification.provenance_coverage,
    )


def _resolve_memory(
    payload: VerifySqlRequest,
    identity: Identity,
    run_id: str,
    store: MemoryStore,
) -> tuple[list[MemoryObject], list[str], list[str]]:
    if payload.memory_ids:
        items: list[MemoryObject] = []
        filtered_permission_ids: list[str] = []
        warnings: list[str] = []
        for memory_id in payload.memory_ids:
            item = store.get_memory(memory_id)
            if item is None:
                warnings.append(f"Memory object not found: {memory_id}")
            elif not has_required_permissions(item, identity.permissions):
                filtered_permission_ids.append(memory_id)
            else:
                items.append(item)
        return items, filtered_permission_ids, warnings

    parsed = parse_task(payload.task_text, run_id)
    plan = plan_task(parsed, provenance_level=3)
    result = retrieve(
        RetrieveRequest(
            task_text=payload.task_text,
            required_types=plan.required_memory_types,  # type: ignore[arg-type]
            time_range=payload.time_range or parsed.time_range,
            user_permissions=identity.permissions,
            max_items=payload.max_items,
        ),
        store=store,
    )
    return result.items, result.filtered_permission_ids, result.warnings


def _first_memory(items: list[MemoryObject], memory_type: str) -> MemoryObject | None:
    for item in items:
        if item.type == memory_type:
            return item
    return None
