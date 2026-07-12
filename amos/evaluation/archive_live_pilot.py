"""Archive a previously executed live-model pilot into a paper bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def archive_live_pilot(source_dir: str | Path, output_dir: str | Path) -> dict[str, Any]:
    source = Path(source_dir).resolve()
    output = Path(output_dir).resolve()
    results_source = source / "results.json"
    summary_source = source / "summary.md"
    if not results_source.exists():
        raise FileNotFoundError(f"Live-pilot results are missing: {results_source}")
    payload = json.loads(results_source.read_text(encoding="utf-8"))
    _validate_pilot_payload(payload)
    output.mkdir(parents=True, exist_ok=True)
    trace_dir = output / "raw_traces"
    trace_dir.mkdir(parents=True, exist_ok=True)
    archived_traces = []
    seen_trace_paths: set[Path] = set()
    for trial in payload.get("live_agent_trials", {}).get("trials", []):
        for attempt in [*trial.get("attempt_history", []), trial]:
            raw_value = attempt.get("raw_trace_path")
            if not raw_value:
                continue
            raw_path = Path(str(raw_value)).resolve()
            if not raw_path.exists() or not raw_path.is_file():
                raise FileNotFoundError(f"Live-pilot raw trace is missing: {raw_path}")
            destination = trace_dir / raw_path.name
            if raw_path not in seen_trace_paths:
                shutil.copy2(raw_path, destination)
                archived_traces.append(_file_record(destination, output))
                seen_trace_paths.add(raw_path)
            attempt["source_raw_trace_path"] = str(raw_path)
            attempt["archived_raw_trace_path"] = str(destination.relative_to(output))

    results_path = output / "results.json"
    results_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if summary_source.exists():
        shutil.copy2(summary_source, output / "summary.md")
    else:
        (output / "summary.md").write_text(_summary(payload), encoding="utf-8")

    files = [_file_record(results_path, output), _file_record(output / "summary.md", output), *archived_traces]
    manifest = {
        "archive_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source_directory": str(source),
        "status": payload["status"],
        "provider": payload.get("provider"),
        "model": payload.get("model"),
        "policy_trials_completed": payload.get("policy_trials", {}).get("completed", 0),
        "live_agent_trials_completed": payload.get("live_agent_trials", {}).get("completed", 0),
        "provider_failures": payload.get("provider_failures", 0),
        "provider_attempt_failures": payload.get("provider_attempt_failures", payload.get("provider_failures", 0)),
        "files": files,
        "evidence_boundary": (
            "This archive preserves a feasibility pilot and all recorded provider-attempt failures. Even a "
            "completed pilot is not evidence of live-model robustness, independent grading, or population-level performance."
        ),
    }
    (output / "archive_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def _validate_pilot_payload(payload: dict[str, Any]) -> None:
    required = ["status", "provider", "model", "policy_trials", "live_agent_trials"]
    missing = [key for key in required if key not in payload]
    if missing:
        raise ValueError(f"Live-pilot results are missing required fields: {missing}")
    if payload["status"] not in {"completed", "partial"}:
        raise ValueError("Only completed or partial live-pilot results can be archived as executed evidence.")
    policy = payload["policy_trials"]
    agent = payload["live_agent_trials"]
    if int(policy.get("completed", 0)) > len(policy.get("trials", [])):
        raise ValueError("Policy completed count exceeds archived trials.")
    if int(agent.get("completed", 0)) > len(agent.get("trials", [])):
        raise ValueError("Live-agent completed count exceeds archived trials.")


def _file_record(path: Path, root: Path) -> dict[str, Any]:
    data = path.read_bytes()
    return {
        "path": str(path.relative_to(root)),
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def _summary(payload: dict[str, Any]) -> str:
    policy = payload["policy_trials"]
    agent = payload["live_agent_trials"]
    return "\n".join(
        [
            "# AMOS Live-Model Feasibility Pilot",
            "",
            f"- Status: {payload['status']}",
            f"- Provider: {payload['provider']}",
            f"- Model: {payload['model']}",
            f"- Policy graded: {policy.get('graded_passed', 0)}/{policy.get('completed', 0)}",
            f"- End-to-end completed: {agent.get('completed', 0)}/{len(agent.get('trials', []))}",
            f"- Provider failures: {payload.get('provider_failures', 0)}",
            "",
            "This feasibility pilot is not a robustness study.",
            "",
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Archive an executed AMOS live-model pilot.")
    parser.add_argument("source_dir")
    parser.add_argument("output_dir")
    args = parser.parse_args()
    print(json.dumps(archive_live_pilot(args.source_dir, args.output_dir), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
