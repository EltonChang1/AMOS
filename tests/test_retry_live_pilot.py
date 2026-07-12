from __future__ import annotations

import json
from types import SimpleNamespace

from amos.evaluation.retry_live_pilot import retry_failed_live_agent_trials


class FakeProvider:
    provider_name = "provider-a"
    model = "model-a"


def test_retry_only_replaces_provider_failures_and_preserves_attempt(tmp_path, monkeypatch) -> None:
    failed_trace = tmp_path / "failed.json"
    failed_trace.write_text('{"status":"error"}', encoding="utf-8")
    successful_trace = tmp_path / "success.json"
    successful_trace.write_text('{"status":"warning"}', encoding="utf-8")
    source = tmp_path / "source.json"
    source.write_text(
        json.dumps(
            {
                "status": "partial",
                "provider": "provider-a",
                "model": "model-a",
                "provider_failures": 1,
                "policy_trials": {
                    "completed": 1,
                    "graded_passed": 1,
                    "trials": [{"status": "completed", "graded_pass": True}],
                },
                "live_agent_trials": {
                    "completed": 1,
                    "graded_passed": 1,
                    "trials": [
                        {
                            "trial_id": "live_agent_001",
                            "prompt": "already complete",
                            "status": "warning",
                            "graded_pass": True,
                            "raw_trace_path": str(successful_trace),
                        },
                        {
                            "trial_id": "live_agent_002",
                            "prompt": "retry me",
                            "status": "error",
                            "graded_pass": False,
                            "raw_trace_path": str(failed_trace),
                        },
                    ],
                },
            }
        ),
        encoding="utf-8",
    )
    calls = []
    monkeypatch.setattr("amos.evaluation.retry_live_pilot.seed_memory", lambda **kwargs: None)
    monkeypatch.setattr("amos.evaluation.retry_live_pilot.seed_duckdb", lambda: None)

    def successful_retry(prompt, user, *, provider, provenance_level):
        calls.append(prompt)
        return SimpleNamespace(
            status="warning",
            verification_status="warning",
            provider="provider-a",
            model="model-a",
            raw_trace_path=str(successful_trace),
        )

    monkeypatch.setattr("amos.evaluation.retry_live_pilot.run_live_agent_task", successful_retry)
    output = tmp_path / "retry" / "results.json"
    result = retry_failed_live_agent_trials(source, output, provider=FakeProvider())

    assert calls == ["retry me"]
    assert result["status"] == "completed"
    assert result["provider_failures"] == 0
    assert result["provider_attempt_failures"] == 1
    assert result["live_agent_trials"]["completed"] == 2
    retried = result["live_agent_trials"]["trials"][1]
    assert retried["graded_pass"] is True
    assert retried["attempt_history"][0]["status"] == "error"
    assert "End-to-end completed: 2/2" in output.with_name("summary.md").read_text(encoding="utf-8")
