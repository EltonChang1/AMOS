from __future__ import annotations

import hashlib
import json
from pathlib import Path

from amos.evaluation.claim_annotation_evidence import (
    score_claim_predictions,
    validate_claim_annotation_study,
)


TEXT = "Failure rate rose to 7.4%. The deployment may have contributed."


def _label(text: str, claim_type: str, requires_review: bool, evidence: list[str]) -> dict[str, object]:
    start = TEXT.index(text)
    return {
        "span_start": start,
        "span_end": start + len(text),
        "text": text,
        "claim_type": claim_type,
        "requires_review": requires_review,
        "evidence_requirements": evidence,
    }


def _manifest(tmp_path: Path, *, synthetic: bool = False, cross_split: bool = False) -> Path:
    raw_dir = tmp_path / "raw"
    raw_dir.mkdir()
    first_raw = raw_dir / "artifact-001.txt"
    first_raw.write_text(TEXT, encoding="utf-8")
    labels = [
        _label("Failure rate rose to 7.4%.", "numeric", False, ["query", "metric", "data_state"]),
        _label("The deployment may have contributed.", "causal", True, ["document", "review"]),
    ]
    artifacts = [
        {
            "artifact_id": "artifact-001",
            "source_group_id": "source-001",
            "author_group_id": "author-001",
            "domain": "payments",
            "artifact_kind": "report",
            "split": "test",
            "synthetic": synthetic,
            "raw_path": "raw/artifact-001.txt",
            "raw_sha256": hashlib.sha256(first_raw.read_bytes()).hexdigest(),
            "annotations": [
                {"annotator_id": "annotator-a", "labels": labels},
                {"annotator_id": "annotator-b", "labels": labels},
            ],
            "adjudicator_id": "adjudicator-c",
            "adjudicated_labels": [{**label, "rationale": "Resolved by guideline section 3."} for label in labels],
        }
    ]
    if cross_split:
        second_raw = raw_dir / "artifact-002.txt"
        second_raw.write_text(TEXT, encoding="utf-8")
        artifacts.append(
            {
                **artifacts[0],
                "artifact_id": "artifact-002",
                "author_group_id": "author-002",
                "split": "development",
                "raw_path": "raw/artifact-002.txt",
                "raw_sha256": hashlib.sha256(second_raw.read_bytes()).hexdigest(),
            }
        )
    payload = {
        "study_id": "claim-study-001",
        "protocol_version": 1,
        "preregistration_sha256": "a" * 64,
        "annotation_guidelines_sha256": "b" * 64,
        "implementation_team_ids": ["amos-author"],
        "artifacts": artifacts,
    }
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps(payload), encoding="utf-8")
    return manifest


def test_claim_annotation_manifest_validates_agreement_but_not_scale_gate(tmp_path: Path) -> None:
    result = validate_claim_annotation_study(
        _manifest(tmp_path),
        strict_min_artifacts=2,
        strict_min_adjudicated_claims=3,
    )
    assert result["structurally_admissible"] is True
    assert result["completion_gate_met"] is False
    assert result["agreement"]["micro_exact_span_f1"] == 1.0
    assert result["agreement"]["mean_claim_type_kappa_on_matched_spans"] == 1.0
    assert result["test_claim_count"] == 2


def test_claim_manifest_rejects_synthetic_and_split_leakage(tmp_path: Path) -> None:
    result = validate_claim_annotation_study(
        _manifest(tmp_path, synthetic=True, cross_split=True),
        strict_min_artifacts=1,
        strict_min_adjudicated_claims=1,
    )
    assert result["structurally_admissible"] is False
    assert any("synthetic" in error for error in result["errors"])
    assert any("crosses data splits" in error for error in result["errors"])


def test_claim_predictions_score_spans_types_review_and_evidence(tmp_path: Path) -> None:
    manifest = _manifest(tmp_path)
    predictions = {
        "system_id": "structured-llm-v1",
        "predictions": [
            {"artifact_id": "artifact-001", **_label("Failure rate rose to 7.4%.", "numeric", False, ["query", "metric", "data_state"])},
            {"artifact_id": "artifact-001", **_label("The deployment may have contributed.", "causal", True, ["document", "review"])},
        ],
    }
    predictions_path = tmp_path / "predictions.json"
    predictions_path.write_text(json.dumps(predictions), encoding="utf-8")

    result = score_claim_predictions(manifest, predictions_path)
    assert result["statistical_unit"] == "source artifact"
    assert result["exact_span_f1"] == 1.0
    assert result["type_accuracy_on_matched_spans"] == 1.0
    assert result["review_accuracy_on_matched_spans"] == 1.0
    assert result["mean_evidence_f1_on_matched_spans"] == 1.0
