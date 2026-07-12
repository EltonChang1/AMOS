from __future__ import annotations

import json
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.config import settings
from amos.memory.seed_memory import seed_memory
from amos.memory.store import MemoryStore
from amos.verifier.metric_checks import check_metric_rules
from amos.verifier.schema_checks import check_schema
from amos.verifier.sql_checks import check_sql_read_only


FIXTURE_PATH = Path(__file__).resolve().parent / "fixtures" / "verifier_benchmark" / "cases.json"


def run_verifier_benchmark(*, write_artifacts: bool = True) -> dict[str, Any]:
    """Evaluate verifier behavior on a frozen, case-level regression corpus.

    Labels are hand-authored engineering labels and must not be described as
    independent human adjudication. The output explicitly preserves that
    evidence boundary.
    """

    payload = json.loads(FIXTURE_PATH.read_text(encoding="utf-8"))
    seed_memory(reset=True)
    store = MemoryStore()
    schema = store.get_memory("memory_schema_payment_events_v2")
    metric = store.get_memory("memory_metric_payment_failure_rate_v3")
    if schema is None or metric is None:
        raise RuntimeError("Verifier benchmark requires seeded payment schema and metric memory.")

    rows: list[dict[str, Any]] = []
    for case in payload["cases"]:
        sql = str(case["sql"])
        safety = check_sql_read_only(sql)
        schema_errors: list[str] = []
        metric_errors: list[str] = []
        if safety.ok:
            schema_errors = check_schema(sql, schema)[1]
            metric_errors = check_metric_rules(sql, metric)[1]
        observed_pass = safety.ok and not schema_errors and not metric_errors
        expected_pass = bool(case["expected_pass"])
        rows.append(
            {
                **case,
                "observed_pass": observed_pass,
                "correct": observed_pass == expected_pass,
                "safety_errors": safety.errors,
                "schema_errors": schema_errors,
                "metric_errors": metric_errors,
            }
        )

    valid = [row for row in rows if row["expected_pass"]]
    invalid = [row for row in rows if not row["expected_pass"]]
    true_positive = sum(1 for row in invalid if not row["observed_pass"])
    false_negative = sum(1 for row in invalid if row["observed_pass"])
    true_negative = sum(1 for row in valid if row["observed_pass"])
    false_positive = sum(1 for row in valid if not row["observed_pass"])

    categories: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        categories[str(row["category"])].append(row)

    result = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "fixture": str(FIXTURE_PATH),
        "fixture_version": payload["version"],
        "label_source": payload["label_source"],
        "evidence_boundary": (
            "Engineering regression evidence only. Labels were authored with the prototype and are not an "
            "independently adjudicated verifier benchmark."
        ),
        "total": len(rows),
        "valid_cases": len(valid),
        "invalid_cases": len(invalid),
        "true_positive_invalid_rejected": true_positive,
        "false_negative_invalid_accepted": false_negative,
        "true_negative_valid_accepted": true_negative,
        "false_positive_valid_rejected": false_positive,
        "valid_acceptance_rate": round(true_negative / len(valid), 3) if valid else 0.0,
        "invalid_rejection_rate": round(true_positive / len(invalid), 3) if invalid else 0.0,
        "accuracy": round(sum(1 for row in rows if row["correct"]) / len(rows), 3) if rows else 0.0,
        "category_summary": {
            category: {
                "total": len(items),
                "correct": sum(1 for item in items if item["correct"]),
            }
            for category, items in sorted(categories.items())
        },
        "cases": rows,
    }
    if write_artifacts:
        out_dir = settings.artifact_dir / "evaluation" / "verifier_benchmark"
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "results.json").write_text(json.dumps(result, indent=2, sort_keys=True), encoding="utf-8")
        (out_dir / "summary.md").write_text(_summary(result), encoding="utf-8")
    return result


def _summary(result: dict[str, Any]) -> str:
    return "\n".join(
        [
            "# AMOS Verifier Engineering Benchmark",
            "",
            f"- Total cases: {result['total']}",
            f"- Valid acceptance rate: {result['valid_acceptance_rate']}",
            f"- Invalid rejection rate: {result['invalid_rejection_rate']}",
            f"- False-positive valid rejections: {result['false_positive_valid_rejected']}",
            f"- False-negative invalid acceptances: {result['false_negative_invalid_accepted']}",
            "",
            f"Evidence boundary: {result['evidence_boundary']}",
            "",
        ]
    )


if __name__ == "__main__":
    print(json.dumps(run_verifier_benchmark(write_artifacts=True), indent=2, sort_keys=True))
