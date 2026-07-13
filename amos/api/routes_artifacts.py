from __future__ import annotations

from pathlib import Path
from typing import Any

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import (
    ArtifactDetailResponse,
    ArtifactListResponse,
    ArtifactProvenanceResponse,
    api_status,
    new_run_id,
)
from amos.config import settings
from amos.memory.permissions import has_required_permissions
from amos.memory.store import MemoryStore
from amos.provenance.export import export_provenance
from amos.verifier.verifier import verify_provenance


router = APIRouter(prefix="/artifacts", tags=["artifacts"])


@router.get("", response_model=ArtifactListResponse)
def list_artifacts(
    limit: int = 20,
    identity: Identity = Depends(get_identity),
) -> ArtifactListResponse:
    run_id = new_run_id()
    store = MemoryStore()
    created_by = None if {"admin", "reviewer"}.intersection(identity.roles) else identity.user_id
    artifacts = store.list_artifacts(created_by=created_by, limit=limit)
    return ArtifactListResponse(
        **api_context(identity, run_id),
        status="pass",
        artifacts=artifacts,
    )


@router.get("/{artifact_id}", response_model=ArtifactDetailResponse)
def get_artifact(
    artifact_id: str,
    identity: Identity = Depends(get_identity),
) -> ArtifactDetailResponse:
    run_id = new_run_id()
    store = MemoryStore()
    artifact = store.get_artifact(artifact_id)
    if artifact is None:
        return ArtifactDetailResponse(
            **api_context(identity, run_id),
            status="reject",
            warnings=["Artifact not found."],
        )
    if not _can_access_artifact(artifact.created_by, identity):
        return ArtifactDetailResponse(
            **api_context(identity, run_id),
            status="reject",
            warnings=["You do not have access to this artifact."],
        )

    warnings: list[str] = []
    report_markdown = _read_report(artifact.path)
    if not report_markdown:
        warnings.append("The report file is unavailable.")

    claims = store.list_claims(artifact_id)
    raw_provenance = export_provenance(artifact_id, store)
    citations, redactions = _redact_provenance(raw_provenance, identity, store)
    if redactions:
        warnings.append("Restricted provenance entries were redacted.")

    provenance = store.list_claim_provenance(artifact_id)
    verification = verify_provenance(claims, provenance, 3) if claims and provenance else None
    package = store.get_replay_package(artifact_id)
    chart_urls = (
        [f"/artifacts/{artifact_id}/charts/{chart_id}" for chart_id in package.chart_ids]
        if package is not None
        else []
    )
    status = verification.status if verification is not None else "warning"
    if warnings and status == "pass":
        status = "warning"
    return ArtifactDetailResponse(
        **api_context(identity, run_id),
        status=api_status(status),
        artifact=artifact,
        report_markdown=report_markdown,
        claims=claims,
        citations=citations,
        chart_urls=chart_urls,
        provenance_coverage=verification.provenance_coverage if verification is not None else 0.0,
        warnings=warnings,
    )


@router.get("/{artifact_id}/charts/{chart_id}", response_class=FileResponse)
def get_chart(
    artifact_id: str,
    chart_id: str,
    identity: Identity = Depends(get_identity),
) -> FileResponse:
    store = MemoryStore()
    artifact = store.get_artifact(artifact_id)
    if artifact is None:
        raise HTTPException(status_code=404, detail="Artifact not found.")
    if not _can_access_artifact(artifact.created_by, identity):
        raise HTTPException(status_code=403, detail="You do not have access to this artifact.")
    package = store.get_replay_package(artifact_id)
    if package is None or chart_id not in package.chart_ids:
        raise HTTPException(status_code=404, detail="Chart not found.")
    chart_path = (settings.charts_dir / f"{chart_id}.png").resolve()
    if not chart_path.is_file() or not chart_path.is_relative_to(settings.charts_dir.resolve()):
        raise HTTPException(status_code=404, detail="Chart not found.")
    return FileResponse(chart_path, media_type="image/png", filename=f"{chart_id}.png")


@router.get("/{artifact_id}/provenance", response_model=ArtifactProvenanceResponse)
def get_provenance(
    artifact_id: str,
    identity: Identity = Depends(get_identity),
) -> ArtifactProvenanceResponse:
    run_id = new_run_id()
    store = MemoryStore()
    artifact = store.get_artifact(artifact_id)
    if artifact is None:
        return ArtifactProvenanceResponse(
            **api_context(identity, run_id),
            status="reject",
            artifact={},
            claims=[],
            warnings=["Artifact not found."],
        )

    claims, redactions = _redact_provenance(export_provenance(artifact_id, store), identity, store)
    status = "warning" if redactions else "pass"
    if redactions:
        store.log(
            "api.artifact.provenance.redact",
            identity.user_id,
            {"run_id": run_id, "artifact_id": artifact_id},
            {"redactions": redactions},
            "warning",
        )
    store.log(
        "api.artifact.provenance",
        identity.user_id,
        {"run_id": run_id, "artifact_id": artifact_id},
        {"claim_count": len(claims), "redaction_count": len(redactions)},
        status,
    )
    return ArtifactProvenanceResponse(
        **api_context(identity, run_id),
        status=status,
        artifact=artifact.model_dump(mode="json"),
        claims=claims,
        redactions=redactions,
        warnings=["Restricted provenance entries were redacted."] if redactions else [],
    )


def _redact_provenance(
    records: list[dict[str, Any]],
    identity: Identity,
    store: MemoryStore,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    redaction_ids: dict[str, str] = {}
    redactions: list[dict[str, Any]] = []

    def replacement(value: str) -> str:
        item = store.get_memory(value)
        if item is None or has_required_permissions(item, identity.permissions):
            return value
        redaction_id = redaction_ids.get(value)
        if redaction_id is None:
            redaction_id = f"redacted_memory_{len(redaction_ids) + 1}"
            redaction_ids[value] = redaction_id
            redactions.append(
                {
                    "redaction_id": redaction_id,
                    "type": item.type,
                    "sensitivity": item.sensitivity,
                    "reason": "insufficient_permissions",
                }
            )
        return redaction_id

    def redact_list(values: object) -> object:
        if not isinstance(values, list):
            return values
        return [replacement(value) if isinstance(value, str) else value for value in values]

    redacted_records: list[dict[str, Any]] = []
    for record in records:
        redacted = dict(record)
        for key in ["support", "memory_object_ids", "document_refs"]:
            redacted[key] = redact_list(redacted.get(key, []))
        semantic_state = redacted.get("semantic_state")
        if isinstance(semantic_state, dict):
            semantic_state = dict(semantic_state)
            for key in ["metric_definition_ids", "schema_ids", "stream_state_ids"]:
                semantic_state[key] = redact_list(semantic_state.get(key, []))
            redacted["semantic_state"] = semantic_state
        redacted_records.append(redacted)
    return redacted_records, redactions


def _can_access_artifact(created_by: str, identity: Identity) -> bool:
    return created_by == identity.user_id or bool({"admin", "reviewer"}.intersection(identity.roles))


def _read_report(raw_path: str) -> str:
    report_path = Path(raw_path).resolve()
    if not report_path.is_file() or not report_path.is_relative_to(settings.reports_dir.resolve()):
        return ""
    return report_path.read_text(encoding="utf-8")
