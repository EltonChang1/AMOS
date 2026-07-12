from __future__ import annotations

from fastapi import APIRouter, Depends

from amos.api.identity import Identity, api_context, get_identity
from amos.api.schemas import ClaimsCiteRequest, ClaimsCiteResponse, api_status, new_run_id
from amos.memory.store import MemoryStore
from amos.verifier.verifier import verify_provenance


router = APIRouter(prefix="/claims", tags=["claims"])


@router.post("/cite", response_model=ClaimsCiteResponse)
def cite_claims_endpoint(
    payload: ClaimsCiteRequest,
    identity: Identity = Depends(get_identity),
) -> ClaimsCiteResponse:
    run_id = new_run_id()
    store = MemoryStore()
    artifact = store.get_artifact(payload.artifact_id)
    if artifact is None:
        errors = ["Artifact not found."]
        store.log("api.claims.cite", identity.user_id, payload.model_dump(), {"errors": errors}, "reject")
        return ClaimsCiteResponse(
            **api_context(identity, run_id),
            status="reject",
            artifact_id=payload.artifact_id,
            claims=[],
            citations=[],
            errors=errors,
        )

    claims = store.list_claims(payload.artifact_id)
    if payload.claim_ids is not None:
        requested = set(payload.claim_ids)
        claims = [claim for claim in claims if claim.claim_id in requested]
    provenance = store.list_claim_provenance(payload.artifact_id)
    if payload.claim_ids is not None:
        requested = set(payload.claim_ids)
        provenance = [record for record in provenance if record.claim_id in requested]

    if not claims or not provenance:
        warnings = ["No claim citations are available for the requested artifact or claim IDs."]
        store.log("api.claims.cite", identity.user_id, payload.model_dump(), {"warnings": warnings}, "needs_review")
        return ClaimsCiteResponse(
            **api_context(identity, run_id),
            status="needs_review",
            artifact_id=payload.artifact_id,
            claims=claims,
            citations=[record.model_dump(mode="json") for record in provenance],
            warnings=warnings,
        )

    verification = verify_provenance(claims, provenance, payload.provenance_level)
    status = api_status(verification.status)
    store.log(
        "api.claims.cite",
        identity.user_id,
        {"run_id": run_id, "artifact_id": payload.artifact_id, "claim_ids": payload.claim_ids},
        {"status": status, "provenance_coverage": verification.provenance_coverage},
        status,
    )
    return ClaimsCiteResponse(
        **api_context(identity, run_id),
        status=status,
        artifact_id=payload.artifact_id,
        claims=claims,
        citations=[record.model_dump(mode="json") for record in provenance],
        provenance_coverage=verification.provenance_coverage,
        warnings=verification.warnings,
        errors=verification.errors,
    )
