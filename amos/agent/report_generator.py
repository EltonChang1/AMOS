from __future__ import annotations

from pathlib import Path

from amos.memory.models import ClaimRecord, MemoryObject


APPLIED_FEEDBACK_AUTHORITIES = {"owner_approved", "reviewer_approved"}


def generate_report(
    artifact_id: str,
    previous_rate: float,
    current_rate: float,
    top_segment: dict[str, object],
    chart_path: Path,
    claims: list[ClaimRecord],
    memory_items: list[MemoryObject],
    verification_status: str,
    warnings: list[str],
    replay_package_id: str,
) -> str:
    metric = _find(memory_items, "semantic_definition")
    schema = _find(memory_items, "schema")
    stream = _find(memory_items, "stream_state")
    applied_feedback, untrusted_feedback = _split_feedback(memory_items)
    warning_lines = "\n".join(f"- {warning}" for warning in warnings) if warnings else "- None"
    feedback_lines = "\n".join(f"- {item.summary}" for item in applied_feedback) if applied_feedback else "- None"
    untrusted_lines = (
        "\n".join(f"- {item.id} ({item.authority}): {item.summary}" for item in untrusted_feedback)
        if untrusted_feedback
        else "- None"
    )
    claim_lines = "\n".join(f"- {claim.claim_id}: {claim.claim_text}" for claim in claims)
    chart_rel = f"../charts/{chart_path.name}"

    return f"""# Payment Failure Rate Investigation

## Summary
Payment failure rate increased from {previous_rate:.1%} to {current_rate:.1%} between the previous six-hour baseline and the current event-time window.

## Evidence
- Metric used: payment_failure_rate:{metric.content["version"]}
- Metric rule: exclude test accounts and use production payment attempts only.
- Schema used: payment_events:{schema.content["version"]}
- Data state: {stream.content["snapshot_id"]}
- Event-time window: {stream.content["event_time_start"]} to {stream.content["event_time_end"]}
- Watermark: {stream.content["watermark"]}
- Late-data policy: {stream.content["late_data_policy"]}

## Likely Contributors
The increase is concentrated in {top_segment["processor"]} and {top_segment["card_network"]} transactions, where failure rate reached {float(top_segment["failure_rate"]):.1%}. A payment gateway deployment occurred before the increase, but this causal explanation requires human review.

## Dashboard Recommendation
Update the executive dashboard with a warning annotation for the current event-time window, cite the Processor B / Visa concentration, and mark the cause as pending human review rather than final.

## Reviewer Feedback Applied
{feedback_lines}

## Untrusted Evidence Considered
{untrusted_lines}

## Chart
![Failure rate by event-time hour]({chart_rel})

## Provenance
{claim_lines}
- Replay package: {replay_package_id}

## Verification Status
{verification_status}

Warnings:
{warning_lines}
"""


def _find(items: list[MemoryObject], memory_type: str) -> MemoryObject:
    matches = [item for item in items if item.type == memory_type]
    if not matches:
        raise ValueError(f"Missing required memory type {memory_type}")
    return matches[0]


def _split_feedback(items: list[MemoryObject]) -> tuple[list[MemoryObject], list[MemoryObject]]:
    feedback = [item for item in items if item.type == "feedback"]
    applied = [item for item in feedback if item.authority in APPLIED_FEEDBACK_AUTHORITIES]
    untrusted = [item for item in feedback if item.authority not in APPLIED_FEEDBACK_AUTHORITIES]
    return applied, untrusted
