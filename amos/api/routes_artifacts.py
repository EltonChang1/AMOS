from __future__ import annotations

from typing import Any

from fastapi import APIRouter, Depends

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import ArtifactProvenanceResponse, new_run_id
from amos.memory.permissions import has_required_permissions
from amos.memory.store import MemoryStore
from amos.provenance.export import export_provenance


router = APIRouter(prefix="/artifacts", tags=["artifacts"])


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
