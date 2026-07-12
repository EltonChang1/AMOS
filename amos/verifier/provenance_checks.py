from __future__ import annotations

from amos.memory.models import ClaimRecord
from amos.provenance.models import ClaimProvenance


def provenance_coverage(claims: list[ClaimRecord], provenance: list[ClaimProvenance]) -> float:
    important = [claim for claim in claims if claim.claim_type in {"numeric", "causal", "recommendation"}]
    if not important:
        return 1.0
    provenance_by_claim = {prov.claim_id: prov for prov in provenance}
    covered = {
        claim.claim_id
        for claim in important
        if (prov := provenance_by_claim.get(claim.claim_id)) is not None
        and _claim_has_required_support(claim, prov)
    }
    return len([claim for claim in important if claim.claim_id in covered]) / len(important)


def check_provenance_level(
    claims: list[ClaimRecord],
    provenance: list[ClaimProvenance],
    provenance_level: int,
) -> tuple[list[str], list[str], float]:
    coverage = provenance_coverage(claims, provenance)
    warnings: list[str] = []
    errors: list[str] = []
    if provenance_level >= 3 and coverage < 1.0:
        errors.append(f"Claim-level provenance coverage is {coverage:.2f}; expected 1.00.")
    provenance_by_claim = {prov.claim_id: prov for prov in provenance}
    for claim in claims:
        prov = provenance_by_claim.get(claim.claim_id)
        if provenance_level >= 3 and claim.claim_type in {"numeric", "causal", "recommendation"}:
            if prov is None:
                errors.append(f"Claim {claim.claim_id} has no provenance record.")
            else:
                errors.extend(_claim_support_errors(claim, prov))
        if claim.claim_type in {"causal", "recommendation"} and claim.requires_review:
            warnings.append(f"Claim {claim.claim_id} requires human review.")
    return warnings, errors, coverage


def _claim_has_required_support(claim: ClaimRecord, provenance: ClaimProvenance) -> bool:
    return not _claim_support_errors(claim, provenance)


def _claim_support_errors(claim: ClaimRecord, provenance: ClaimProvenance) -> list[str]:
    errors: list[str] = []
    if not provenance.support:
        errors.append(f"Claim {claim.claim_id} has empty support.")
    if not provenance.memory_object_ids:
        errors.append(f"Claim {claim.claim_id} must cite supporting memory objects.")
    if not provenance.semantic_state.get("metric_definition_ids"):
        errors.append(f"Claim {claim.claim_id} must cite a metric definition.")
    if not provenance.semantic_state.get("schema_ids"):
        errors.append(f"Claim {claim.claim_id} must cite a schema version.")

    if claim.claim_id.endswith("_rate_increase"):
        _require_query_kind(claim, provenance, "summary", errors)
        _require_query_kind(claim, provenance, "timeseries", errors)
        if not provenance.chart_ids:
            errors.append(f"Claim {claim.claim_id} must cite the time-series chart.")
    elif claim.claim_id.endswith("_concentration"):
        _require_query_kind(claim, provenance, "concentration", errors)
    elif claim.claim_type == "causal":
        _require_query_kind(claim, provenance, "summary", errors)
        _require_query_kind(claim, provenance, "concentration", errors)
        if not provenance.document_refs:
            errors.append(f"Claim {claim.claim_id} must cite deployment/review document evidence.")
        if not claim.requires_review:
            errors.append(f"Claim {claim.claim_id} must be marked as requiring human review.")
    elif claim.claim_type == "recommendation":
        _require_query_kind(claim, provenance, "summary", errors)
        _require_query_kind(claim, provenance, "concentration", errors)
        if not provenance.document_refs:
            errors.append(f"Claim {claim.claim_id} must cite guidance or document evidence.")
        if not claim.requires_review:
            errors.append(f"Claim {claim.claim_id} must be marked as requiring human review.")
    elif claim.claim_type == "numeric":
        if not provenance.query_ids:
            errors.append(f"Claim {claim.claim_id} must cite a numeric query.")
    return errors


def _require_query_kind(
    claim: ClaimRecord,
    provenance: ClaimProvenance,
    kind: str,
    errors: list[str],
) -> None:
    if not any(query_id.endswith(f"_{kind}") for query_id in provenance.query_ids):
        errors.append(f"Claim {claim.claim_id} must cite the {kind} query.")
