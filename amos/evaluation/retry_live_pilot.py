"""Retry only unresolved provider failures from an archived live-model pilot."""

from __future__ import annotations

import argparse
import json
from copy import deepcopy
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.agent.live_agent import LiveLLMProvider, provider_from_env, run_live_agent_task
from amos.evaluation.run_live_pilot import _summary_text
from amos.memory.models import User
from amos.memory.seed_memory import seed_memory
from amos.tools.seed_duckdb import seed_duckdb


ANALYST_PERMISSIONS = ["analytics", "payments"]
COMPLETED_AGENT_STATUSES = {"pass", "warning", "reject", "completed"}
FAILED_PROVIDER_STATUSES = {"error", "failed"}


def retry_failed_live_agent_trials(
    source: str | Path,
    output: str | Path,
    *,
    provider: LiveLLMProvider | None = None,
) -> dict[str, Any]:
    source_path = Path(source).resolve()
    output_path = Path(output).resolve()
    payload = json.loads(source_path.read_text(encoding="utf-8"))
    if "live_agent_trials" not in payload or "policy_trials" not in payload:
        raise ValueError("Source is not a live-pilot result payload.")

    provider = provider or provider_from_env()
    seed_memory(reset=True)
    seed_duckdb()
    retried = 0
    retry_failures = 0
    trials = payload["live_agent_trials"].get("trials", [])
    for index, trial in enumerate(list(trials)):
        if trial.get("status") not in FAILED_PROVIDER_STATUSES:
            continue
        retried += 1
        previous = deepcopy(trial)
        prior_history = list(previous.pop("attempt_history", []))
        try:
            result = run_live_agent_task(
                str(trial["prompt"]),
                User(id="analyst_001", permissions=ANALYST_PERMISSIONS),
                provider=provider,
                provenance_level=3,
            )
            status = result.status
            verification = result.verification_status
            replacement = {
                "trial_id": trial["trial_id"],
                "prompt": trial["prompt"],
                "status": status,
                "verification_status": verification,
                "provider": result.provider,
                "model": result.model,
                "raw_trace_path": result.raw_trace_path,
                "graded_pass": bool(
                    status in {"pass", "warning"}
                    and verification in {"pass", "warning", None}
                ),
            }
        except Exception as exc:  # pragma: no cover - preserved in external retry evidence
            retry_failures += 1
            replacement = {
                "trial_id": trial["trial_id"],
                "prompt": trial["prompt"],
                "status": "failed",
                "error": repr(exc),
                "graded_pass": False,
            }
        if replacement.get("status") in FAILED_PROVIDER_STATUSES:
            retry_failures += int("error" not in replacement or not isinstance(replacement.get("error"), str))
        replacement["attempt_number"] = len(prior_history) + 2
        replacement["attempt_history"] = [*prior_history, previous]
        trials[index] = replacement

    _recompute(payload, retry_failures=retry_failures)
    payload["retry_provenance"] = {
        "source_results": str(source_path),
        "retried_at": datetime.now(timezone.utc).isoformat(),
        "retried_live_agent_trials": retried,
        "provider": provider.provider_name,
        "model": provider.model,
        "preserves_failed_attempts": True,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    output_path.with_name("summary.md").write_text(_summary_text(payload), encoding="utf-8")
    return payload


def _recompute(payload: dict[str, Any], *, retry_failures: int) -> None:
    policy = payload["policy_trials"]
    agents = payload["live_agent_trials"]
    policy_trials = policy.get("trials", [])
    agent_trials = agents.get("trials", [])
    policy_completed = sum(1 for trial in policy_trials if trial.get("status") == "completed")
    policy_passed = sum(1 for trial in policy_trials if trial.get("graded_pass"))
    agent_completed = sum(1 for trial in agent_trials if trial.get("status") in COMPLETED_AGENT_STATUSES)
    agent_passed = sum(1 for trial in agent_trials if trial.get("graded_pass"))
    unresolved_failures = sum(1 for trial in policy_trials if trial.get("status") != "completed") + sum(
        1 for trial in agent_trials if trial.get("status") in FAILED_PROVIDER_STATUSES
    )
    historical_failures = int(payload.get("provider_attempt_failures", payload.get("provider_failures", 0)))

    policy.update(
        intended=len(policy_trials),
        completed=policy_completed,
        graded_passed=policy_passed,
        graded_pass_rate=round(policy_passed / policy_completed, 3) if policy_completed else 0.0,
    )
    agents.update(
        intended=len(agent_trials),
        completed=agent_completed,
        graded_passed=agent_passed,
        graded_pass_rate=round(agent_passed / agent_completed, 3) if agent_completed else 0.0,
    )
    payload["status"] = (
        "completed"
        if policy_completed == len(policy_trials) and agent_completed == len(agent_trials)
        else "partial"
    )
    payload["samples_completed"] = policy_completed + agent_completed
    payload["provider_failures"] = unresolved_failures
    payload["provider_attempt_failures"] = historical_failures + retry_failures
    payload["graded_passed"] = policy_passed + agent_passed
    payload["graded_pass_rate"] = round(
        (policy_passed + agent_passed) / max(policy_completed + agent_completed, 1), 3
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Retry unresolved live-agent provider failures only.")
    parser.add_argument("source")
    parser.add_argument("output")
    args = parser.parse_args()
    result = retry_failed_live_agent_trials(args.source, args.output)
    print(
        json.dumps(
            {
                "output": str(Path(args.output).resolve()),
                "status": result["status"],
                "provider_failures": result["provider_failures"],
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
