from __future__ import annotations

from amos.evaluation.run_live_pilot import _summary_text


def test_summary_reports_intended_trials_and_provider_failures() -> None:
    summary = _summary_text(
        {
            "status": "partial",
            "provider": "provider-a",
            "model": "model-a",
            "provider_failures": 3,
            "policy_trials": {
                "intended": 8,
                "completed": 8,
                "graded_passed": 3,
                "trials": [{} for _ in range(8)],
            },
            "live_agent_trials": {
                "intended": 3,
                "completed": 0,
                "graded_passed": 0,
                "trials": [{} for _ in range(3)],
            },
        }
    )

    assert "Policy completed: 8/8" in summary
    assert "End-to-end completed: 0/3" in summary
    assert "End-to-end graded among completed: 0/0" in summary
    assert "Provider failures: 3" in summary
    assert "Provider-attempt failures preserved: 3" in summary
