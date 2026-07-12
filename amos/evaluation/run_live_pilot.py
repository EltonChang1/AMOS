from __future__ import annotations

import argparse
import json
from pathlib import Path

from amos.config import settings
from amos.evaluation.extended_experiments import run_live_llm_experiment


def _summary_text(result: dict) -> str:
    policy = result.get("policy_trials", {})
    agent = result.get("live_agent_trials", {})
    policy_intended = policy.get("intended", len(policy.get("trials", [])))
    agent_intended = agent.get("intended", len(agent.get("trials", [])))
    return "\n".join(
        [
            "# AMOS Live-Model Feasibility Pilot",
            "",
            f"- Status: {result.get('status')}",
            f"- Provider: {result.get('provider', 'n/a')}",
            f"- Model: {result.get('model', 'n/a')}",
            f"- Policy completed: {policy.get('completed', 0)}/{policy_intended}",
            f"- Policy graded among completed: {policy.get('graded_passed', 0)}/{policy.get('completed', 0)}",
            f"- End-to-end completed: {agent.get('completed', 0)}/{agent_intended}",
            f"- End-to-end graded among completed: {agent.get('graded_passed', 0)}/{agent.get('completed', 0)}",
            f"- Provider failures: {result.get('provider_failures', 0)}",
            f"- Provider-attempt failures preserved: {result.get('provider_attempt_failures', result.get('provider_failures', 0))}",
            "",
            "Evidence boundary: feasibility pilot only; prompts are not independent population samples and the lexical policy grader is not human adjudication.",
            "",
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Run and archive the AMOS live-model feasibility pilot.")
    parser.add_argument("--samples", type=int, default=1)
    parser.add_argument(
        "--output",
        default=None,
        help="Output JSON path (default: artifacts/evaluation/live_llm_pilot/results.json).",
    )
    args = parser.parse_args()
    result = run_live_llm_experiment(samples=max(args.samples, 1))
    output = (
        Path(args.output).resolve()
        if args.output
        else settings.artifact_dir / "evaluation" / "live_llm_pilot" / "results.json"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True), encoding="utf-8")
    summary = output.with_name("summary.md")
    summary.write_text(_summary_text(result), encoding="utf-8")
    print(json.dumps({"output": str(output), "summary": str(summary), "status": result.get("status")}, indent=2))


if __name__ == "__main__":
    main()
