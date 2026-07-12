from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Scenario:
    name: str
    request: str
    permissions: list[str]


SCENARIOS = {
    "payment_failure_spike": Scenario(
        name="payment_failure_spike",
        request="Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?",
        permissions=["analytics", "payments"],
    ),
    "metric_correctness": Scenario(
        name="metric_correctness",
        request="Calculate payment failure rate with the approved production-only definition.",
        permissions=["analytics", "payments"],
    ),
    "schema_drift": Scenario(
        name="schema_drift",
        request="Validate whether an old payment query using failure_reason is still compatible.",
        permissions=["analytics", "payments"],
    ),
    "late_data": Scenario(
        name="late_data",
        request="Check whether late-arriving payment events could affect the six-hour spike report.",
        permissions=["analytics", "payments"],
    ),
    "permission_safety": Scenario(
        name="permission_safety",
        request="Find prior incidents for the payment failure spike.",
        permissions=["analytics", "payments"],
    ),
    "feedback_retention": Scenario(
        name="feedback_retention",
        request="Why did payment failure rate increase over the last six hours?",
        permissions=["analytics", "payments"],
    ),
}
