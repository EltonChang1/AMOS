from __future__ import annotations

from amos.memory.models import ClaimRecord


def build_claims(
    artifact_id: str,
    previous_rate: float,
    current_rate: float,
    top_processor: str,
    top_network: str,
    dashboard_recommendation: str,
) -> list[ClaimRecord]:
    return [
        ClaimRecord(
            claim_id=f"claim_{artifact_id}_rate_increase",
            artifact_id=artifact_id,
            claim_text=(
                f"Payment failure rate increased from {previous_rate:.1%} to {current_rate:.1%} "
                "between the previous and current six-hour event-time windows."
            ),
            claim_type="numeric",
            requires_review=False,
        ),
        ClaimRecord(
            claim_id=f"claim_{artifact_id}_concentration",
            artifact_id=artifact_id,
            claim_text=f"The increase is concentrated in {top_processor} and {top_network} transactions.",
            claim_type="numeric",
            requires_review=False,
        ),
        ClaimRecord(
            claim_id=f"claim_{artifact_id}_deployment",
            artifact_id=artifact_id,
            claim_text=(
                "A payment-gateway deployment occurred before the spike; treating it as a likely contributor "
                "requires human review."
            ),
            claim_type="causal",
            requires_review=True,
        ),
        ClaimRecord(
            claim_id=f"claim_{artifact_id}_dashboard",
            artifact_id=artifact_id,
            claim_text=dashboard_recommendation,
            claim_type="recommendation",
            requires_review=True,
        ),
    ]
