from __future__ import annotations

from fastapi import APIRouter, Depends

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import TaskRunRequest, TaskRunResponse, api_status, new_run_id
from amos.agent.controller import run_amos_task


router = APIRouter(prefix="/tasks", tags=["tasks"])


@router.post("/run", response_model=TaskRunResponse)
def run_task(
    payload: TaskRunRequest,
    identity: Identity = Depends(get_identity),
) -> TaskRunResponse:
    run_id = new_run_id()
    warnings: list[str] = []
    if payload.permissions is not None:
        warnings.append("Ignored client-supplied permissions; using server-side AMOS identity.")
    try:
        result = run_amos_task(
            request=payload.request,
            user=identity.as_user(),
            provenance_level=payload.provenance_level,
        )
    except RuntimeError as exc:
        return TaskRunResponse(
            **api_context(identity, run_id),
            status="reject",
            warnings=warnings,
            errors=[str(exc)],
        )

    warnings.extend(result.warnings)
    status = api_status(result.verification_status)
    if warnings and status == "pass":
        status = "warning"
    return TaskRunResponse(
        **api_context(identity, run_id),
        status=status,
        result=result,
        task_id=result.task_id,
        artifact_id=result.artifact_id,
        replay_package_id=result.replay_package_id,
        warnings=warnings,
    )
