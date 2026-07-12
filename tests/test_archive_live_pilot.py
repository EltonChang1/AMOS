from __future__ import annotations

import json

from amos.evaluation.archive_live_pilot import archive_live_pilot


def test_live_pilot_archive_preserves_partial_failures_and_raw_traces(tmp_path) -> None:
    source = tmp_path / "source"
    source.mkdir()
    trace = source / "trace.json"
    trace.write_text('{"status":"provider_error"}', encoding="utf-8")
    payload = {
        "status": "partial",
        "provider": "provider-a",
        "model": "model-a",
        "provider_failures": 1,
        "policy_trials": {"completed": 1, "graded_passed": 1, "trials": [{"status": "completed"}]},
        "live_agent_trials": {
            "completed": 0,
            "graded_passed": 0,
            "trials": [{"status": "failed", "raw_trace_path": str(trace)}],
        },
    }
    (source / "results.json").write_text(json.dumps(payload), encoding="utf-8")
    (source / "summary.md").write_text("# Pilot\n", encoding="utf-8")

    output = tmp_path / "archive"
    result = archive_live_pilot(source, output)
    archived = json.loads((output / "results.json").read_text(encoding="utf-8"))
    assert result["status"] == "partial"
    assert result["provider_failures"] == 1
    assert len(result["files"]) == 3
    assert archived["live_agent_trials"]["trials"][0]["archived_raw_trace_path"] == "raw_traces/trace.json"
    assert (output / "archive_manifest.json").exists()


def test_live_pilot_archive_preserves_retry_attempt_history(tmp_path) -> None:
    source = tmp_path / "source"
    source.mkdir()
    failed_trace = source / "failed.json"
    failed_trace.write_text('{"status":"error"}', encoding="utf-8")
    passed_trace = source / "passed.json"
    passed_trace.write_text('{"status":"warning"}', encoding="utf-8")
    payload = {
        "status": "completed",
        "provider": "provider-a",
        "model": "model-a",
        "provider_failures": 0,
        "provider_attempt_failures": 1,
        "policy_trials": {"completed": 1, "graded_passed": 1, "trials": [{"status": "completed"}]},
        "live_agent_trials": {
            "completed": 1,
            "graded_passed": 1,
            "trials": [
                {
                    "status": "warning",
                    "raw_trace_path": str(passed_trace),
                    "attempt_history": [{"status": "error", "raw_trace_path": str(failed_trace)}],
                }
            ],
        },
    }
    (source / "results.json").write_text(json.dumps(payload), encoding="utf-8")

    output = tmp_path / "archive"
    manifest = archive_live_pilot(source, output)
    archived = json.loads((output / "results.json").read_text(encoding="utf-8"))

    assert manifest["provider_attempt_failures"] == 1
    assert len(manifest["files"]) == 4
    trial = archived["live_agent_trials"]["trials"][0]
    assert trial["archived_raw_trace_path"] == "raw_traces/passed.json"
    assert trial["attempt_history"][0]["archived_raw_trace_path"] == "raw_traces/failed.json"
