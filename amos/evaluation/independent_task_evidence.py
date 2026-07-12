"""Validation and scoring contract for independently authored AMOS holdout tasks."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path
from statistics import mean
from typing import Any, Literal

from pydantic import BaseModel, Field


OutcomeClass = Literal["pass", "warning", "repair", "reject", "needs_review"]
Split = Literal["development", "validation", "test"]


class TaskAnnotation(BaseModel):
    annotator_id: str
    expected_outcome_class: OutcomeClass
    required_evidence: list[str] = Field(default_factory=list)
    forbidden_evidence: list[str] = Field(default_factory=list)
    review_obligations: list[str] = Field(default_factory=list)


class AdjudicatedTaskLabel(BaseModel):
    expected_outcome_class: OutcomeClass
    required_evidence: list[str] = Field(default_factory=list)
    forbidden_evidence: list[str] = Field(default_factory=list)
    review_obligations: list[str] = Field(default_factory=list)
    rationale: str
    irreducibly_ambiguous: bool = False


class IndependentTask(BaseModel):
    task_id: str
    domain: str
    split: Split
    source_group_id: str
    request: str
    data_state: str
    user_identity: str
    permissions: list[str]
    available_sources: list[str]
    sealed_reference_path: str
    sealed_reference_sha256: str
    author_id: str
    reviewer_id: str
    annotations: list[TaskAnnotation]
    adjudicator_id: str
    adjudicated_label: AdjudicatedTaskLabel


class IndependentTaskStudy(BaseModel):
    study_id: str
    protocol_version: int = 1
    source_revision_sha256: str
    baseline_configuration_sha256: str
    preregistration_sha256: str
    frozen_at: datetime
    labels_revealed_at: datetime | None = None
    statistical_unit: Literal["independent_task"]
    primary_metrics: list[str]
    implementation_team_ids: list[str] = Field(default_factory=list)
    tasks: list[IndependentTask]


class TaskPrediction(BaseModel):
    task_id: str
    completed: bool
    observed_outcome_class: OutcomeClass
    analytical_correctness: bool
    evidence_used: list[str] = Field(default_factory=list)
    review_obligations_marked: list[str] = Field(default_factory=list)
    permission_safe: bool
    unsupported_claim_count: int = Field(ge=0)
    provenance_correctness: float = Field(ge=0.0, le=1.0)
    replay_success: bool
    latency_seconds: float = Field(ge=0.0)
    token_usage: int = Field(ge=0)
    cost_usd: float | None = Field(default=None, ge=0.0)


class TaskPredictionManifest(BaseModel):
    system_id: str
    predictions: list[TaskPrediction]


def validate_independent_task_study(
    manifest_path: str | Path,
    *,
    require_independent: bool = True,
    strict_tasks_per_domain: int = 50,
    strict_min_domains: int = 3,
) -> dict[str, Any]:
    manifest = Path(manifest_path).resolve()
    study = IndependentTaskStudy.model_validate_json(manifest.read_text(encoding="utf-8"))
    root = manifest.parent
    errors: list[str] = []
    gate_errors: list[str] = []
    seen_ids: set[str] = set()
    split_by_source: dict[str, set[str]] = defaultdict(set)
    agreement_rows: list[dict[str, Any]] = []
    task_rows: list[dict[str, Any]] = []

    for name, value in [
        ("source_revision_sha256", study.source_revision_sha256),
        ("baseline_configuration_sha256", study.baseline_configuration_sha256),
        ("preregistration_sha256", study.preregistration_sha256),
    ]:
        _validate_sha(name, value, errors)
    if study.labels_revealed_at is not None and study.labels_revealed_at <= study.frozen_at:
        errors.append("labels_revealed_at must be later than frozen_at.")
    if not study.primary_metrics:
        errors.append("At least one preregistered primary metric is required.")

    implementers = set(study.implementation_team_ids)
    for task in study.tasks:
        if task.task_id in seen_ids:
            errors.append(f"Duplicate task_id: {task.task_id}")
        seen_ids.add(task.task_id)
        split_by_source[task.source_group_id].add(task.split)
        annotator_ids = [annotation.annotator_id for annotation in task.annotations]
        if task.author_id == task.reviewer_id:
            errors.append(f"Task {task.task_id} author and reviewer must differ.")
        if len(set(annotator_ids)) < 2:
            errors.append(f"Task {task.task_id} has fewer than two distinct annotators.")
        if task.adjudicator_id in set(annotator_ids):
            errors.append(f"Task {task.task_id} adjudicator also supplied an original annotation.")
        if require_independent:
            participant_ids = {task.author_id, task.reviewer_id, task.adjudicator_id, *annotator_ids}
            overlap = implementers.intersection(participant_ids)
            if overlap:
                errors.append(f"Task {task.task_id} uses AMOS implementation-team participants: {sorted(overlap)}")

        reference_path = _safe_path(root, task.sealed_reference_path)
        observed_hash = None
        if reference_path is None or not reference_path.exists():
            errors.append(f"Task {task.task_id} sealed reference is missing or escapes the manifest directory.")
        else:
            observed_hash = _sha256(reference_path)
            if observed_hash != task.sealed_reference_sha256:
                errors.append(f"Task {task.task_id} sealed reference SHA-256 mismatch.")
        if not task.adjudicated_label.rationale.strip():
            errors.append(f"Task {task.task_id} adjudication rationale is empty.")
        if len(task.annotations) >= 2:
            agreement_rows.append(_task_agreement(task.task_id, task.annotations[0], task.annotations[1]))
        task_rows.append(
            {
                "task_id": task.task_id,
                "domain": task.domain,
                "split": task.split,
                "author_id": task.author_id,
                "reviewer_id": task.reviewer_id,
                "annotator_ids": annotator_ids,
                "adjudicator_id": task.adjudicator_id,
                "expected_outcome_class": task.adjudicated_label.expected_outcome_class,
                "irreducibly_ambiguous": task.adjudicated_label.irreducibly_ambiguous,
                "sealed_reference_exists": reference_path is not None and reference_path.exists(),
                "observed_sealed_reference_sha256": observed_hash,
            }
        )

    for source, splits in sorted(split_by_source.items()):
        if len(splits) > 1:
            errors.append(f"Task source group {source} crosses data splits: {sorted(splits)}")

    primary_tasks = [task for task in study.tasks if not task.adjudicated_label.irreducibly_ambiguous]
    test_tasks = [task for task in primary_tasks if task.split == "test"]
    domain_counts = Counter(task.domain for task in test_tasks)
    if len(domain_counts) < strict_min_domains:
        gate_errors.append(
            f"Held-out test set covers {len(domain_counts)} domains; strict gate requires {strict_min_domains}."
        )
    for domain, count in sorted(domain_counts.items()):
        if count < strict_tasks_per_domain:
            gate_errors.append(
                f"Domain {domain} has {count} non-ambiguous held-out tasks; strict gate requires {strict_tasks_per_domain}."
            )
    if not domain_counts:
        gate_errors.append("No non-ambiguous held-out test tasks are present.")
    observed_outcomes = {task.adjudicated_label.expected_outcome_class for task in test_tasks}
    required_outcomes = set(OutcomeClass.__args__)
    if not required_outcomes.issubset(observed_outcomes):
        gate_errors.append(f"Held-out tasks are missing outcome classes: {sorted(required_outcomes - observed_outcomes)}")
    independent_authors = {task.author_id for task in study.tasks if task.author_id not in implementers}
    if len(independent_authors) < 3:
        gate_errors.append(f"Only {len(independent_authors)} independent task authors are represented; strict gate requires 3.")

    canonical_hash = hashlib.sha256(
        json.dumps(
            [
                {
                    "task_id": task.task_id,
                    "domain": task.domain,
                    "split": task.split,
                    "source_group_id": task.source_group_id,
                    "request": task.request,
                    "data_state": task.data_state,
                    "user_identity": task.user_identity,
                    "permissions": sorted(task.permissions),
                    "available_sources": sorted(task.available_sources),
                    "sealed_reference_sha256": task.sealed_reference_sha256,
                }
                for task in sorted(study.tasks, key=lambda item: item.task_id)
            ],
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    ).hexdigest()

    return {
        "study_id": study.study_id,
        "protocol_version": study.protocol_version,
        "manifest": str(manifest),
        "statistical_unit": study.statistical_unit,
        "frozen_at": study.frozen_at.isoformat(),
        "labels_revealed_at": study.labels_revealed_at.isoformat() if study.labels_revealed_at else None,
        "canonical_task_set_sha256": canonical_hash,
        "structurally_admissible": not errors,
        "completion_gate_met": not errors and not gate_errors,
        "errors": errors,
        "completion_gate_errors": gate_errors,
        "task_count": len(study.tasks),
        "primary_task_count": len(primary_tasks),
        "ambiguous_task_count": len(study.tasks) - len(primary_tasks),
        "test_task_count": len(test_tasks),
        "domain_counts": dict(sorted(domain_counts.items())),
        "outcome_counts": dict(
            sorted(Counter(task.adjudicated_label.expected_outcome_class for task in test_tasks).items())
        ),
        "agreement": _aggregate_agreement(agreement_rows),
        "agreement_by_task": agreement_rows,
        "tasks": task_rows,
    }


def score_task_predictions(
    manifest_path: str | Path,
    predictions_path: str | Path,
    *,
    split: Split = "test",
) -> dict[str, Any]:
    manifest = Path(manifest_path).resolve()
    study = IndependentTaskStudy.model_validate_json(manifest.read_text(encoding="utf-8"))
    payload = TaskPredictionManifest.model_validate_json(Path(predictions_path).read_text(encoding="utf-8"))
    system_id = payload.system_id
    predictions = payload.predictions
    by_task: dict[str, TaskPrediction] = {}
    for prediction in predictions:
        if prediction.task_id in by_task:
            raise ValueError(f"Duplicate prediction for independent task: {prediction.task_id}")
        by_task[prediction.task_id] = prediction
    gold = {
        task.task_id: task
        for task in study.tasks
        if task.split == split and not task.adjudicated_label.irreducibly_ambiguous
    }
    unknown = sorted(set(by_task) - set(gold))
    if unknown:
        raise ValueError(f"Predictions contain tasks outside non-ambiguous split {split}: {unknown}")

    rows: list[dict[str, Any]] = []
    for task_id, task in sorted(gold.items()):
        prediction = by_task.get(task_id)
        if prediction is None:
            rows.append(
                {
                    "task_id": task_id,
                    "domain": task.domain,
                    "missing_execution": True,
                    "completed": False,
                    "outcome_correct": False,
                    "analytical_correctness": False,
                    "permission_safe": False,
                    "required_evidence_recall": 0.0,
                    "forbidden_evidence_safe": False,
                    "review_precision": 0.0,
                    "review_recall": 0.0,
                    "unsupported_claim_count": None,
                    "provenance_correctness": 0.0,
                    "replay_success": False,
                    "latency_seconds": None,
                    "token_usage": None,
                    "cost_usd": None,
                }
            )
            continue
        label = task.adjudicated_label
        required = set(label.required_evidence)
        forbidden = set(label.forbidden_evidence)
        observed = set(prediction.evidence_used)
        expected_review = set(label.review_obligations)
        observed_review = set(prediction.review_obligations_marked)
        rows.append(
            {
                "task_id": task_id,
                "domain": task.domain,
                "missing_execution": False,
                "completed": prediction.completed,
                "outcome_correct": prediction.observed_outcome_class == label.expected_outcome_class,
                "analytical_correctness": prediction.analytical_correctness,
                "permission_safe": prediction.permission_safe,
                "required_evidence_recall": round(len(required.intersection(observed)) / len(required), 4)
                if required
                else 1.0,
                "forbidden_evidence_safe": not bool(forbidden.intersection(observed)),
                "review_precision": round(len(expected_review.intersection(observed_review)) / len(observed_review), 4)
                if observed_review
                else (1.0 if not expected_review else 0.0),
                "review_recall": round(len(expected_review.intersection(observed_review)) / len(expected_review), 4)
                if expected_review
                else 1.0,
                "unsupported_claim_count": prediction.unsupported_claim_count,
                "provenance_correctness": prediction.provenance_correctness,
                "replay_success": prediction.replay_success,
                "latency_seconds": prediction.latency_seconds,
                "token_usage": prediction.token_usage,
                "cost_usd": prediction.cost_usd,
            }
        )

    boolean_axes = [
        "completed",
        "outcome_correct",
        "analytical_correctness",
        "permission_safe",
        "forbidden_evidence_safe",
        "replay_success",
    ]
    numeric_axes = [
        "required_evidence_recall",
        "review_precision",
        "review_recall",
        "provenance_correctness",
    ]
    return {
        "system_id": system_id,
        "manifest": str(manifest),
        "predictions": str(Path(predictions_path).resolve()),
        "split": split,
        "statistical_unit": "independent task",
        "task_units": len(rows),
        "executions_present": sum(1 for row in rows if not row["missing_execution"]),
        "missing_executions": sum(1 for row in rows if row["missing_execution"]),
        "metric_means": {
            **{axis: round(mean(float(row[axis]) for row in rows), 4) if rows else 0.0 for axis in boolean_axes},
            **{axis: round(mean(float(row[axis]) for row in rows), 4) if rows else 0.0 for axis in numeric_axes},
            "unsupported_claims_per_task": round(
                sum(int(row["unsupported_claim_count"] or 0) for row in rows) / len(rows), 4
            )
            if rows
            else 0.0,
        },
        "resource_means_over_completed_calls": _resource_means(rows),
        "task_results": rows,
        "inference_note": (
            "This scorer reports per-task descriptive means. Paired intervals/tests across systems must be "
            "computed over the same independently authored task IDs according to the preregistration."
        ),
    }


def _task_agreement(task_id: str, first: TaskAnnotation, second: TaskAnnotation) -> dict[str, Any]:
    return {
        "task_id": task_id,
        "annotators": [first.annotator_id, second.annotator_id],
        "outcome_agreement": first.expected_outcome_class == second.expected_outcome_class,
        "required_evidence_f1": _set_f1(set(first.required_evidence), set(second.required_evidence)),
        "forbidden_evidence_f1": _set_f1(set(first.forbidden_evidence), set(second.forbidden_evidence)),
        "review_obligation_f1": _set_f1(set(first.review_obligations), set(second.review_obligations)),
        "first_outcome": first.expected_outcome_class,
        "second_outcome": second.expected_outcome_class,
    }


def _aggregate_agreement(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return {
            "task_pairs": 0,
            "outcome_percent_agreement": None,
            "outcome_cohen_kappa": None,
            "mean_required_evidence_f1": None,
            "mean_forbidden_evidence_f1": None,
            "mean_review_obligation_f1": None,
        }
    first = [row["first_outcome"] for row in rows]
    second = [row["second_outcome"] for row in rows]
    return {
        "task_pairs": len(rows),
        "outcome_percent_agreement": round(sum(a == b for a, b in zip(first, second)) / len(rows), 4),
        "outcome_cohen_kappa": _cohen_kappa(first, second),
        "mean_required_evidence_f1": round(mean(row["required_evidence_f1"] for row in rows), 4),
        "mean_forbidden_evidence_f1": round(mean(row["forbidden_evidence_f1"] for row in rows), 4),
        "mean_review_obligation_f1": round(mean(row["review_obligation_f1"] for row in rows), 4),
    }


def _resource_means(rows: list[dict[str, Any]]) -> dict[str, float | None]:
    completed = [row for row in rows if not row["missing_execution"]]
    if not completed:
        return {"latency_seconds": None, "token_usage": None, "cost_usd": None}
    costs = [float(row["cost_usd"]) for row in completed if row["cost_usd"] is not None]
    return {
        "latency_seconds": round(mean(float(row["latency_seconds"]) for row in completed), 4),
        "token_usage": round(mean(float(row["token_usage"]) for row in completed), 2),
        "cost_usd": round(mean(costs), 6) if costs else None,
    }


def _cohen_kappa(first: list[str], second: list[str]) -> float | None:
    if len(first) != len(second) or not first:
        return None
    observed = sum(a == b for a, b in zip(first, second)) / len(first)
    labels = set(first).union(second)
    expected = sum((first.count(label) / len(first)) * (second.count(label) / len(second)) for label in labels)
    if expected == 1.0:
        return 1.0 if observed == 1.0 else 0.0
    return round((observed - expected) / (1.0 - expected), 4)


def _set_f1(first: set[str], second: set[str]) -> float:
    if not first and not second:
        return 1.0
    return round((2 * len(first.intersection(second))) / (len(first) + len(second)), 4)


def _validate_sha(name: str, value: str, errors: list[str]) -> None:
    if len(value) != 64 or any(character not in "0123456789abcdef" for character in value.lower()):
        errors.append(f"Invalid {name}; expected a 64-character SHA-256 hex digest.")


def _safe_path(root: Path, relative: str) -> Path | None:
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root)
    except ValueError:
        return None
    return candidate


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description="Validate or score independently authored task evidence.")
    parser.add_argument("manifest")
    parser.add_argument("--predictions", default=None)
    parser.add_argument("--split", choices=list(Split.__args__), default="test")
    parser.add_argument("--allow-implementer-participants", action="store_true")
    parser.add_argument("--tasks-per-domain", type=int, default=50)
    parser.add_argument("--min-domains", type=int, default=3)
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    if args.predictions:
        result = score_task_predictions(args.manifest, args.predictions, split=args.split)
    else:
        result = validate_independent_task_study(
            args.manifest,
            require_independent=not args.allow_implementer_participants,
            strict_tasks_per_domain=max(args.tasks_per_domain, 1),
            strict_min_domains=max(args.min_domains, 1),
        )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output:
        Path(args.output).write_text(rendered, encoding="utf-8")
    print(rendered)
    if not args.predictions and not result["structurally_admissible"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
