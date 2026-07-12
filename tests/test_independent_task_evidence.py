from __future__ import annotations

import hashlib
import json
from pathlib import Path

from amos.evaluation.independent_task_evidence import (
    score_task_predictions,
    validate_independent_task_study,
)


def _manifest(tmp_path: Path, *, cross_split: bool = False, use_implementer: bool = False) -> Path:
    sealed = tmp_path / "sealed"
    sealed.mkdir()
    reference = sealed / "task-001.json"
    reference.write_text('{"reference_sql":"SELECT 1"}', encoding="utf-8")
    annotation = {
        "expected_outcome_class": "warning",
        "required_evidence": ["metric-v3", "snapshot-42"],
        "forbidden_evidence": ["restricted-incident"],
        "review_obligations": ["causal-claim"],
    }
    task = {
        "task_id": "task-001",
        "domain": "payments",
        "split": "test",
        "source_group_id": "source-001",
        "request": "Why did failures rise?",
        "data_state": "snapshot-42",
        "user_identity": "analyst-001",
        "permissions": ["analytics", "payments"],
        "available_sources": ["metric-v3", "snapshot-42", "restricted-incident"],
        "sealed_reference_path": "sealed/task-001.json",
        "sealed_reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "author_id": "amos-author" if use_implementer else "analyst-author",
        "reviewer_id": "analyst-reviewer",
        "annotations": [
            {"annotator_id": "annotator-a", **annotation},
            {"annotator_id": "annotator-b", **annotation},
        ],
        "adjudicator_id": "adjudicator-c",
        "adjudicated_label": {
            **annotation,
            "rationale": "Watermark lag requires a warning and causal review.",
            "irreducibly_ambiguous": False,
        },
    }
    tasks = [task]
    if cross_split:
        second_reference = sealed / "task-002.json"
        second_reference.write_text('{"reference_sql":"SELECT 2"}', encoding="utf-8")
        tasks.append(
            {
                **task,
                "task_id": "task-002",
                "split": "development",
                "sealed_reference_path": "sealed/task-002.json",
                "sealed_reference_sha256": hashlib.sha256(second_reference.read_bytes()).hexdigest(),
            }
        )
    payload = {
        "study_id": "holdout-001",
        "protocol_version": 1,
        "source_revision_sha256": "a" * 64,
        "baseline_configuration_sha256": "b" * 64,
        "preregistration_sha256": "c" * 64,
        "frozen_at": "2026-07-01T00:00:00Z",
        "labels_revealed_at": "2026-07-10T00:00:00Z",
        "statistical_unit": "independent_task",
        "primary_metrics": ["task_completion", "analytical_correctness", "permission_safety"],
        "implementation_team_ids": ["amos-author"],
        "tasks": tasks,
    }
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps(payload), encoding="utf-8")
    return manifest


def test_holdout_manifest_validates_freeze_hashes_and_agreement(tmp_path: Path) -> None:
    result = validate_independent_task_study(
        _manifest(tmp_path),
        strict_tasks_per_domain=1,
        strict_min_domains=1,
    )
    assert result["structurally_admissible"] is True
    assert result["completion_gate_met"] is False
    assert result["canonical_task_set_sha256"]
    assert result["agreement"]["outcome_percent_agreement"] == 1.0
    assert result["agreement"]["outcome_cohen_kappa"] == 1.0
    assert any("missing outcome classes" in error.lower() for error in result["completion_gate_errors"])


def test_holdout_manifest_rejects_implementer_authorship_and_split_leakage(tmp_path: Path) -> None:
    result = validate_independent_task_study(
        _manifest(tmp_path, cross_split=True, use_implementer=True),
        strict_tasks_per_domain=1,
        strict_min_domains=1,
    )
    assert result["structurally_admissible"] is False
    assert any("implementation-team" in error for error in result["errors"])
    assert any("crosses data splits" in error for error in result["errors"])


def test_holdout_predictions_score_task_axes_and_missing_executions(tmp_path: Path) -> None:
    manifest = _manifest(tmp_path)
    predictions = {
        "system_id": "amos-live-model-a",
        "predictions": [
            {
                "task_id": "task-001",
                "completed": True,
                "observed_outcome_class": "warning",
                "analytical_correctness": True,
                "evidence_used": ["metric-v3", "snapshot-42"],
                "review_obligations_marked": ["causal-claim"],
                "permission_safe": True,
                "unsupported_claim_count": 0,
                "provenance_correctness": 1.0,
                "replay_success": True,
                "latency_seconds": 2.0,
                "token_usage": 500,
                "cost_usd": 0.02,
            }
        ],
    }
    predictions_path = tmp_path / "predictions.json"
    predictions_path.write_text(json.dumps(predictions), encoding="utf-8")

    result = score_task_predictions(manifest, predictions_path)
    assert result["statistical_unit"] == "independent task"
    assert result["missing_executions"] == 0
    assert result["metric_means"]["completed"] == 1.0
    assert result["metric_means"]["outcome_correct"] == 1.0
    assert result["metric_means"]["required_evidence_recall"] == 1.0
    assert result["metric_means"]["forbidden_evidence_safe"] == 1.0
