from __future__ import annotations

import json
from pathlib import Path

import pytest

from amos.evaluation.paired_task_analysis import compare_paired_task_scores


def _row(task_id: str, passed: bool, latency: float) -> dict[str, object]:
    return {
        "task_id": task_id,
        "completed": passed,
        "outcome_correct": passed,
        "analytical_correctness": passed,
        "permission_safe": True,
        "forbidden_evidence_safe": True,
        "replay_success": passed,
        "required_evidence_recall": 1.0 if passed else 0.5,
        "review_precision": 1.0,
        "review_recall": 1.0 if passed else 0.0,
        "provenance_correctness": 1.0 if passed else 0.0,
        "unsupported_claim_count": 0 if passed else 1,
        "latency_seconds": latency,
        "token_usage": 100,
        "cost_usd": 0.01,
    }


def _score_file(tmp_path: Path, name: str, passes: list[bool]) -> Path:
    path = tmp_path / f"{name}.json"
    payload = {
        "system_id": name,
        "split": "test",
        "statistical_unit": "independent task",
        "task_results": [_row(f"task-{index}", passed, 1.0 + index) for index, passed in enumerate(passes)],
    }
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def test_paired_analysis_uses_same_task_units_and_reports_mcnemar(tmp_path: Path) -> None:
    first = _score_file(tmp_path, "baseline", [False, False, True, True])
    second = _score_file(tmp_path, "amos", [True, True, True, True])

    result = compare_paired_task_scores(first, second, bootstrap_samples=500, seed=7)
    completed = result["axes"]["completed"]
    assert result["paired_task_count"] == 4
    assert result["difference_direction"] == "second_system_minus_first_system"
    assert completed["mean_difference"] == 0.5
    assert completed["discordant_first_only"] == 0
    assert completed["discordant_second_only"] == 2
    assert completed["mcnemar_exact_two_sided_p"] == 0.5
    assert completed["task_bootstrap_interval95"]["lower"] >= 0.0


def test_paired_analysis_rejects_mismatched_task_ids(tmp_path: Path) -> None:
    first = _score_file(tmp_path, "baseline", [False, True])
    second = _score_file(tmp_path, "amos", [True])
    with pytest.raises(ValueError, match="Paired task IDs differ"):
        compare_paired_task_scores(first, second, bootstrap_samples=10)
