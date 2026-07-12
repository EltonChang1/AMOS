"""Paired descriptive and inferential analysis over independent task units."""

from __future__ import annotations

import argparse
import json
import math
import random
from pathlib import Path
from statistics import mean, stdev
from typing import Any


BOOLEAN_AXES = [
    "completed",
    "outcome_correct",
    "analytical_correctness",
    "permission_safe",
    "forbidden_evidence_safe",
    "replay_success",
]
NUMERIC_AXES = [
    "required_evidence_recall",
    "review_precision",
    "review_recall",
    "provenance_correctness",
    "unsupported_claim_count",
    "latency_seconds",
    "token_usage",
    "cost_usd",
]


def compare_paired_task_scores(
    first_path: str | Path,
    second_path: str | Path,
    *,
    bootstrap_samples: int = 10_000,
    seed: int = 20260712,
) -> dict[str, Any]:
    """Compare two systems on the exact same independently authored task IDs.

    Differences are second minus first. Missing executions remain scored on the
    boolean/task-quality axes by the upstream scorer; resource axes use only
    pairs with observed values for both systems.
    """

    first_file = Path(first_path).resolve()
    second_file = Path(second_path).resolve()
    first = json.loads(first_file.read_text(encoding="utf-8"))
    second = json.loads(second_file.read_text(encoding="utf-8"))
    if first.get("statistical_unit") != "independent task" or second.get("statistical_unit") != "independent task":
        raise ValueError("Both score files must declare 'independent task' as the statistical unit.")
    if first.get("split") != second.get("split"):
        raise ValueError("Paired score files must use the same data split.")

    first_rows = _rows_by_task(first)
    second_rows = _rows_by_task(second)
    if set(first_rows) != set(second_rows):
        missing_first = sorted(set(second_rows) - set(first_rows))
        missing_second = sorted(set(first_rows) - set(second_rows))
        raise ValueError(
            f"Paired task IDs differ; missing from first={missing_first}, missing from second={missing_second}."
        )
    task_ids = sorted(first_rows)
    rng = random.Random(seed)
    axes: dict[str, Any] = {}
    for axis in BOOLEAN_AXES:
        pairs = [(float(bool(first_rows[task][axis])), float(bool(second_rows[task][axis]))) for task in task_ids]
        axes[axis] = {
            **_paired_summary(pairs, bootstrap_samples=bootstrap_samples, rng=rng),
            "mcnemar_exact_two_sided_p": _mcnemar_exact(pairs),
            "discordant_first_only": sum(1 for first_value, second_value in pairs if first_value == 1 and second_value == 0),
            "discordant_second_only": sum(1 for first_value, second_value in pairs if first_value == 0 and second_value == 1),
        }
    for axis in NUMERIC_AXES:
        pairs = [
            (float(first_rows[task][axis]), float(second_rows[task][axis]))
            for task in task_ids
            if first_rows[task].get(axis) is not None and second_rows[task].get(axis) is not None
        ]
        axes[axis] = _paired_summary(pairs, bootstrap_samples=bootstrap_samples, rng=rng)

    return {
        "first_system": first.get("system_id"),
        "second_system": second.get("system_id"),
        "first_score_file": str(first_file),
        "second_score_file": str(second_file),
        "split": first.get("split"),
        "statistical_unit": "independent task",
        "paired_task_count": len(task_ids),
        "task_ids_sha256": _task_ids_hash(task_ids),
        "difference_direction": "second_system_minus_first_system",
        "bootstrap_samples": bootstrap_samples,
        "bootstrap_seed": seed,
        "axes": axes,
        "analysis_note": (
            "Intervals resample independent task IDs. McNemar's exact test is reported for binary axes. "
            "The caller remains responsible for preregistered multiplicity correction and hierarchical "
            "analysis when stochastic runs are nested within tasks."
        ),
    }


def _rows_by_task(payload: dict[str, Any]) -> dict[str, dict[str, Any]]:
    rows: dict[str, dict[str, Any]] = {}
    for row in payload.get("task_results", []):
        task_id = str(row["task_id"])
        if task_id in rows:
            raise ValueError(f"Duplicate task unit in score file: {task_id}")
        rows[task_id] = row
    return rows


def _paired_summary(
    pairs: list[tuple[float, float]],
    *,
    bootstrap_samples: int,
    rng: random.Random,
) -> dict[str, Any]:
    if not pairs:
        return {
            "paired_units": 0,
            "first_mean": None,
            "second_mean": None,
            "mean_difference": None,
            "paired_effect_size_dz": None,
            "task_bootstrap_interval95": None,
        }
    differences = [second - first for first, second in pairs]
    interval = _bootstrap_mean_interval(differences, bootstrap_samples, rng)
    deviation = stdev(differences) if len(differences) > 1 else 0.0
    return {
        "paired_units": len(pairs),
        "first_mean": round(mean(first for first, _ in pairs), 6),
        "second_mean": round(mean(second for _, second in pairs), 6),
        "mean_difference": round(mean(differences), 6),
        "paired_effect_size_dz": round(mean(differences) / deviation, 6) if deviation else None,
        "task_bootstrap_interval95": {"lower": interval[0], "upper": interval[1]},
    }


def _bootstrap_mean_interval(
    differences: list[float],
    samples: int,
    rng: random.Random,
) -> tuple[float, float]:
    if not differences:
        return 0.0, 0.0
    if len(differences) == 1:
        value = round(differences[0], 6)
        return value, value
    estimates = []
    for _ in range(max(samples, 1)):
        estimates.append(mean(rng.choice(differences) for _ in differences))
    estimates.sort()
    lower = estimates[int(0.025 * (len(estimates) - 1))]
    upper = estimates[int(0.975 * (len(estimates) - 1))]
    return round(lower, 6), round(upper, 6)


def _mcnemar_exact(pairs: list[tuple[float, float]]) -> float | None:
    first_only = sum(1 for first, second in pairs if first == 1 and second == 0)
    second_only = sum(1 for first, second in pairs if first == 0 and second == 1)
    discordant = first_only + second_only
    if discordant == 0:
        return 1.0
    tail = sum(math.comb(discordant, value) for value in range(min(first_only, second_only) + 1)) / (2**discordant)
    return round(min(1.0, 2 * tail), 6)


def _task_ids_hash(task_ids: list[str]) -> str:
    import hashlib

    return hashlib.sha256("\n".join(task_ids).encode("utf-8")).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description="Run paired analysis over two independent-task score files.")
    parser.add_argument("first")
    parser.add_argument("second")
    parser.add_argument("--bootstrap-samples", type=int, default=10_000)
    parser.add_argument("--seed", type=int, default=20260712)
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    result = compare_paired_task_scores(
        args.first,
        args.second,
        bootstrap_samples=max(args.bootstrap_samples, 1),
        seed=args.seed,
    )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output:
        Path(args.output).write_text(rendered, encoding="utf-8")
    print(rendered)


if __name__ == "__main__":
    main()
