from __future__ import annotations

from fastapi import APIRouter, Depends

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import ReplayApiResponse, api_status, new_run_id
from amos.memory.store import MemoryStore
from amos.provenance.replay import ReplayResult, replay_artifact


router = APIRouter(prefix="/artifacts", tags=["replay"])


@router.post("/{artifact_id}/replay", response_model=ReplayApiResponse)
def replay(
    artifact_id: str,
    identity: Identity = Depends(get_identity),
) -> ReplayApiResponse:
    run_id = new_run_id()
    result: ReplayResult = replay_artifact(artifact_id)
    store = MemoryStore()
    store.log(
        "api.artifact.replay",
        identity.user_id,
        {"run_id": run_id, "artifact_id": artifact_id},
        {"status": result.status, "warnings": result.warnings, "errors": result.errors},
        result.status,
    )
    return ReplayApiResponse(
        **api_context(identity, run_id),
        status=api_status(result.status),
        artifact_id=artifact_id,
        replay_status=result.status,
        warnings=result.warnings,
        errors=result.errors,
    )
