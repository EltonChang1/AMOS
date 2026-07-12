from __future__ import annotations

import uuid
from datetime import datetime, timezone

from fastapi import APIRouter, Depends
from pydantic import BaseModel

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import (
    MemoryReconcileRequest,
    MemoryReconcileResponse,
    MemoryRetrieveRequest,
    MemoryRetrieveResponse,
    MemorySupersedeRequest,
    MemorySupersedeResponse,
    MemoryWriteRequest,
    MemoryWriteResponse,
    api_status,
    new_run_id,
)
from amos.agent.controller import write_feedback
from amos.memory.models import MemoryObject, RetrieveRequest, RetrieveResult
from amos.memory.reconciliation import reconcile
from amos.memory.retrieval import retrieve
from amos.memory.store import MemoryStore


router = APIRouter(prefix="/memory", tags=["memory"])


class FeedbackRequest(BaseModel):
    artifact_id: str
    reviewer_role: str
    feedback: str
    authority: str = "reviewer_approved"
    effective_start: datetime | None = None


@router.post("/retrieve", response_model=MemoryRetrieveResponse)
def retrieve_memory(
    payload: MemoryRetrieveRequest,
    identity: Identity = Depends(get_identity),
) -> MemoryRetrieveResponse:
    run_id = new_run_id()
    store = MemoryStore()
    warnings: list[str] = []
    if payload.user_permissions is not None:
        warnings.append("Ignored client-supplied permissions; using server-side AMOS identity.")

    result = retrieve(
        RetrieveRequest(
            task_text=payload.task_text,
            required_types=payload.required_types,
            time_range=payload.time_range,
            user_permissions=identity.permissions,
            max_items=payload.max_items,
        ),
        store=store,
    )
    warnings.extend(result.warnings)
    status = "warning" if warnings else "pass"
    store.log(
        "api.memory.retrieve",
        identity.user_id,
        {
            "run_id": run_id,
            "task_text": payload.task_text,
            "required_types": payload.required_types,
            "tenant_id": identity.tenant_id,
            "project_id": identity.project_id,
        },
        {
            "returned_ids": [item.id for item in result.items],
            "filtered_permission_ids": result.filtered_permission_ids,
            "warnings": warnings,
        },
        status,
    )
    return MemoryRetrieveResponse(
        **api_context(identity, run_id),
        status=api_status(status),
        items=result.items,
        filtered_permission_ids=result.filtered_permission_ids,
        warnings=warnings,
        memory_version_ids=[item.version for item in result.items],
    )


@router.post("/write", response_model=MemoryWriteResponse)
def write_memory(
    payload: MemoryWriteRequest,
    identity: Identity = Depends(get_identity),
) -> MemoryWriteResponse:
    run_id = new_run_id()
    store = MemoryStore()
    if payload.authority == "owner_approved" and "admin" not in identity.roles:
        errors = ["Only admin identities can create owner-approved memory through the dev API."]
        store.log("api.memory.write", identity.user_id, payload.model_dump(mode="json"), {"errors": errors}, "reject")
        return MemoryWriteResponse(**api_context(identity, run_id), status="reject", errors=errors)

    version = payload.version or datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")
    item = MemoryObject(
        id=payload.memory_id or f"memory_{uuid.uuid4().hex[:12]}",
        type=payload.type,
        summary=payload.summary,
        content=payload.content,
        source=payload.source,
        authority=payload.authority,
        effective_start=payload.effective_start,
        effective_end=payload.effective_end,
        permissions=payload.permissions if payload.permissions is not None else identity.permissions,
        sensitivity=payload.sensitivity,
        version=version,
        status=payload.status,
        supersedes=payload.supersedes,
        provenance_ref=payload.provenance_ref,
    )
    store.upsert_memory(item)
    store.log(
        "api.memory.write",
        identity.user_id,
        {"run_id": run_id, "memory_id": item.id},
        {"status": item.status, "memory_version_id": item.version},
        "pass",
    )
    return MemoryWriteResponse(
        **api_context(identity, run_id),
        status="pass",
        item=item,
        memory_version_id=item.version,
    )


@router.post("/supersede", response_model=MemorySupersedeResponse)
def supersede_memory(
    payload: MemorySupersedeRequest,
    identity: Identity = Depends(get_identity),
) -> MemorySupersedeResponse:
    run_id = new_run_id()
    store = MemoryStore()
    errors: list[str] = []
    if store.get_memory(payload.old_memory_id) is None:
        errors.append(f"Memory object not found: {payload.old_memory_id}")
    if store.get_memory(payload.new_memory_id) is None:
        errors.append(f"Memory object not found: {payload.new_memory_id}")
    if errors:
        store.log("api.memory.supersede", identity.user_id, payload.model_dump(), {"errors": errors}, "reject")
        return MemorySupersedeResponse(
            **api_context(identity, run_id),
            status="reject",
            old_memory_id=payload.old_memory_id,
            new_memory_id=payload.new_memory_id,
            errors=errors,
        )

    store.supersede(payload.old_memory_id, payload.new_memory_id)
    store.log(
        "api.memory.supersede",
        identity.user_id,
        {"run_id": run_id, **payload.model_dump()},
        {"status": "superseded"},
        "pass",
    )
    return MemorySupersedeResponse(
        **api_context(identity, run_id),
        status="pass",
        old_memory_id=payload.old_memory_id,
        new_memory_id=payload.new_memory_id,
    )


@router.post("/reconcile", response_model=MemoryReconcileResponse)
def reconcile_memory(
    payload: MemoryReconcileRequest,
    identity: Identity = Depends(get_identity),
) -> MemoryReconcileResponse:
    run_id = new_run_id()
    store = MemoryStore()
    if payload.memory_ids is None:
        candidates = store.list_memory()
    else:
        candidates = [item for memory_id in payload.memory_ids if (item := store.get_memory(memory_id)) is not None]
    items, warnings = reconcile(candidates)
    status = "warning" if warnings else "pass"
    store.log(
        "api.memory.reconcile",
        identity.user_id,
        {"run_id": run_id, "memory_ids": payload.memory_ids},
        {"selected_ids": [item.id for item in items], "warnings": warnings},
        status,
    )
    return MemoryReconcileResponse(
        **api_context(identity, run_id),
        status=api_status(status),
        items=items,
        warnings=warnings,
    )


@router.post("/feedback", response_model=MemoryObject)
def feedback(payload: FeedbackRequest) -> MemoryObject:
    return write_feedback(
        artifact_id=payload.artifact_id,
        reviewer_role=payload.reviewer_role,
        feedback=payload.feedback,
        authority=payload.authority,
        effective_start=payload.effective_start,
    )
