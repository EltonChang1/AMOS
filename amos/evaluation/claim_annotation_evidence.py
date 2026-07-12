"""Validate and summarize independently annotated claim-extraction evidence.

The validator keeps structural admissibility separate from the paper's strict
completion gate. Small fixtures can therefore test the import contract without
being mistaken for a publication-scale independent corpus.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
from statistics import mean
from typing import Any, Literal

from pydantic import BaseModel, Field


ClaimType = Literal["numeric", "causal", "recommendation", "context", "forecast", "comparison"]
ArtifactKind = Literal[
    "report",
    "notebook",
    "slide",
    "chart_annotation",
    "table_cell",
    "dashboard_text",
    "fragment",
]
Split = Literal["development", "validation", "test"]


class ClaimLabel(BaseModel):
    span_start: int = Field(ge=0)
    span_end: int = Field(gt=0)
    text: str
    claim_type: ClaimType
    requires_review: bool
    evidence_requirements: list[str] = Field(default_factory=list)


class AnnotationSet(BaseModel):
    annotator_id: str
    labels: list[ClaimLabel]


class AdjudicatedClaim(ClaimLabel):
    rationale: str


class ClaimArtifact(BaseModel):
    artifact_id: str
    source_group_id: str
    author_group_id: str
    domain: str
    artifact_kind: ArtifactKind
    split: Split
    synthetic: bool
    raw_path: str
    raw_sha256: str
    annotations: list[AnnotationSet]
    adjudicator_id: str
    adjudicated_labels: list[AdjudicatedClaim]


class ClaimAnnotationStudy(BaseModel):
    study_id: str
    protocol_version: int = 1
    preregistration_sha256: str
    annotation_guidelines_sha256: str
    implementation_team_ids: list[str] = Field(default_factory=list)
    artifacts: list[ClaimArtifact]


class ClaimPrediction(ClaimLabel):
    artifact_id: str


class ClaimPredictionManifest(BaseModel):
    system_id: str
    predictions: list[ClaimPrediction]


def validate_claim_annotation_study(
    manifest_path: str | Path,
    *,
    require_independent: bool = True,
    strict_min_artifacts: int = 120,
    strict_min_adjudicated_claims: int = 600,
) -> dict[str, Any]:
    """Validate raw artifacts, leakage boundaries, annotations, and agreement."""

    manifest = Path(manifest_path).resolve()
    study = ClaimAnnotationStudy.model_validate_json(manifest.read_text(encoding="utf-8"))
    root = manifest.parent
    errors: list[str] = []
    gate_errors: list[str] = []
    seen_artifacts: set[str] = set()
    split_by_source: dict[str, set[str]] = defaultdict(set)
    split_by_author: dict[str, set[str]] = defaultdict(set)
    agreement_rows: list[dict[str, Any]] = []
    artifact_rows: list[dict[str, Any]] = []

    _validate_sha("preregistration_sha256", study.preregistration_sha256, errors)
    _validate_sha("annotation_guidelines_sha256", study.annotation_guidelines_sha256, errors)

    for artifact in study.artifacts:
        if artifact.artifact_id in seen_artifacts:
            errors.append(f"Duplicate artifact_id: {artifact.artifact_id}")
        seen_artifacts.add(artifact.artifact_id)
        split_by_source[artifact.source_group_id].add(artifact.split)
        split_by_author[artifact.author_group_id].add(artifact.split)
        if require_independent and artifact.synthetic:
            errors.append(f"Artifact {artifact.artifact_id} is synthetic and cannot support an independent claim corpus.")

        annotator_ids = [annotation.annotator_id for annotation in artifact.annotations]
        if len(set(annotator_ids)) < 2:
            errors.append(f"Artifact {artifact.artifact_id} has fewer than two distinct annotators.")
        if artifact.adjudicator_id in set(annotator_ids):
            errors.append(f"Artifact {artifact.artifact_id} adjudicator also supplied an original annotation.")
        if require_independent:
            prohibited = set(study.implementation_team_ids)
            overlap = prohibited.intersection([*annotator_ids, artifact.adjudicator_id])
            if overlap:
                errors.append(
                    f"Artifact {artifact.artifact_id} uses AMOS implementation-team participants: {sorted(overlap)}"
                )

        raw_path = _safe_path(root, artifact.raw_path)
        raw_text = None
        observed_hash = None
        if raw_path is None or not raw_path.exists():
            errors.append(f"Artifact {artifact.artifact_id} raw file is missing or escapes the manifest directory.")
        else:
            observed_hash = _sha256(raw_path)
            if observed_hash != artifact.raw_sha256:
                errors.append(f"Artifact {artifact.artifact_id} raw SHA-256 mismatch.")
            try:
                raw_text = raw_path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                errors.append(f"Artifact {artifact.artifact_id} is not UTF-8 text; provide a normalized text export.")

        for annotation in artifact.annotations:
            _validate_labels(
                artifact.artifact_id,
                annotation.labels,
                raw_text,
                errors,
                label_source=f"annotator {annotation.annotator_id}",
            )
        _validate_labels(
            artifact.artifact_id,
            artifact.adjudicated_labels,
            raw_text,
            errors,
            label_source=f"adjudicator {artifact.adjudicator_id}",
        )
        if any(not label.rationale.strip() for label in artifact.adjudicated_labels):
            errors.append(f"Artifact {artifact.artifact_id} has an adjudicated label without rationale.")

        if len(artifact.annotations) >= 2:
            agreement_rows.append(
                _artifact_agreement(
                    artifact.artifact_id,
                    artifact.annotations[0],
                    artifact.annotations[1],
                )
            )
        artifact_rows.append(
            {
                "artifact_id": artifact.artifact_id,
                "domain": artifact.domain,
                "artifact_kind": artifact.artifact_kind,
                "split": artifact.split,
                "source_group_id": artifact.source_group_id,
                "author_group_id": artifact.author_group_id,
                "annotator_ids": annotator_ids,
                "adjudicator_id": artifact.adjudicator_id,
                "adjudicated_claims": len(artifact.adjudicated_labels),
                "raw_exists": raw_path is not None and raw_path.exists(),
                "observed_raw_sha256": observed_hash,
            }
        )

    for source, splits in sorted(split_by_source.items()):
        if len(splits) > 1:
            errors.append(f"Source group {source} crosses data splits: {sorted(splits)}")
    for author, splits in sorted(split_by_author.items()):
        if len(splits) > 1:
            errors.append(f"Author group {author} crosses data splits: {sorted(splits)}")

    adjudicated_claims = sum(len(artifact.adjudicated_labels) for artifact in study.artifacts)
    observed_kinds = {artifact.artifact_kind for artifact in study.artifacts}
    required_kinds = set(ArtifactKind.__args__)
    test_claims = sum(
        len(artifact.adjudicated_labels) for artifact in study.artifacts if artifact.split == "test"
    )
    if len(study.artifacts) < strict_min_artifacts:
        gate_errors.append(
            f"Independent claim corpus has {len(study.artifacts)} artifacts; strict gate requires {strict_min_artifacts}."
        )
    if adjudicated_claims < strict_min_adjudicated_claims:
        gate_errors.append(
            f"Independent claim corpus has {adjudicated_claims} adjudicated claims; strict gate requires "
            f"{strict_min_adjudicated_claims}."
        )
    if not required_kinds.issubset(observed_kinds):
        gate_errors.append(f"Missing artifact kinds: {sorted(required_kinds - observed_kinds)}")
    if test_claims == 0:
        gate_errors.append("No adjudicated claims are assigned to the held-out test split.")

    agreement = _aggregate_agreement(agreement_rows)
    split_summary = _split_summary(study.artifacts)
    return {
        "study_id": study.study_id,
        "protocol_version": study.protocol_version,
        "manifest": str(manifest),
        "require_independent": require_independent,
        "structurally_admissible": not errors,
        "completion_gate_met": not errors and not gate_errors,
        "errors": errors,
        "completion_gate_errors": gate_errors,
        "artifact_count": len(study.artifacts),
        "adjudicated_claim_count": adjudicated_claims,
        "test_claim_count": test_claims,
        "artifact_kind_counts": dict(sorted(Counter(a.artifact_kind for a in study.artifacts).items())),
        "domain_counts": dict(sorted(Counter(a.domain for a in study.artifacts).items())),
        "split_summary": split_summary,
        "agreement": agreement,
        "agreement_by_artifact": agreement_rows,
        "artifacts": artifact_rows,
        "statistical_note": (
            "Agreement is computed over independently annotated artifacts before adjudication. "
            "Exact-span F1 measures extraction agreement; kappa is computed only for exact-span matches."
        ),
    }


def score_claim_predictions(
    manifest_path: str | Path,
    predictions_path: str | Path,
    *,
    split: Split = "test",
) -> dict[str, Any]:
    """Score extractor predictions against sealed adjudicated labels by artifact."""

    manifest = Path(manifest_path).resolve()
    study = ClaimAnnotationStudy.model_validate_json(manifest.read_text(encoding="utf-8"))
    predictions_file = Path(predictions_path).resolve()
    payload = ClaimPredictionManifest.model_validate_json(predictions_file.read_text(encoding="utf-8"))
    system_id = payload.system_id
    by_artifact: dict[str, list[ClaimLabel]] = defaultdict(list)
    duplicate_ids: set[str] = set()
    seen_ids: set[tuple[str, int, int]] = set()
    for row in payload.predictions:
        artifact_id = row.artifact_id
        label = ClaimLabel.model_validate(row.model_dump())
        key = (artifact_id, label.span_start, label.span_end)
        if key in seen_ids:
            duplicate_ids.add(f"{artifact_id}:{label.span_start}-{label.span_end}")
        seen_ids.add(key)
        by_artifact[artifact_id].append(label)
    if duplicate_ids:
        raise ValueError(f"Duplicate prediction spans: {sorted(duplicate_ids)}")

    gold_artifacts = {artifact.artifact_id: artifact for artifact in study.artifacts if artifact.split == split}
    unknown = sorted(set(by_artifact) - set(gold_artifacts))
    if unknown:
        raise ValueError(f"Predictions contain artifacts outside split {split}: {unknown}")

    rows = []
    all_gold = 0
    all_predicted = 0
    exact_matches = 0
    typed_matches = 0
    review_matches = 0
    evidence_f1_values: list[float] = []
    for artifact_id, artifact in sorted(gold_artifacts.items()):
        gold = list(artifact.adjudicated_labels)
        predicted = by_artifact.get(artifact_id, [])
        gold_by_span = {_span(label): label for label in gold}
        predicted_by_span = {_span(label): label for label in predicted}
        matches = sorted(set(gold_by_span).intersection(predicted_by_span))
        all_gold += len(gold)
        all_predicted += len(predicted)
        exact_matches += len(matches)
        typed = sum(
            1 for span in matches if gold_by_span[span].claim_type == predicted_by_span[span].claim_type
        )
        review = sum(
            1
            for span in matches
            if gold_by_span[span].requires_review == predicted_by_span[span].requires_review
        )
        typed_matches += typed
        review_matches += review
        artifact_evidence = [
            _set_f1(
                set(gold_by_span[span].evidence_requirements),
                set(predicted_by_span[span].evidence_requirements),
            )
            for span in matches
        ]
        evidence_f1_values.extend(artifact_evidence)
        rows.append(
            {
                "artifact_id": artifact_id,
                "gold_claims": len(gold),
                "predicted_claims": len(predicted),
                "exact_span_matches": len(matches),
                "exact_span_f1": _f1(len(matches), len(predicted), len(gold)),
                "type_accuracy_on_matched_spans": round(typed / len(matches), 4) if matches else None,
                "review_accuracy_on_matched_spans": round(review / len(matches), 4) if matches else None,
                "evidence_f1_on_matched_spans": round(mean(artifact_evidence), 4) if artifact_evidence else None,
            }
        )

    return {
        "system_id": system_id,
        "manifest": str(manifest),
        "predictions": str(predictions_file),
        "split": split,
        "artifact_units": len(gold_artifacts),
        "gold_claims": all_gold,
        "predicted_claims": all_predicted,
        "exact_span_matches": exact_matches,
        "exact_span_precision": round(exact_matches / all_predicted, 4) if all_predicted else 0.0,
        "exact_span_recall": round(exact_matches / all_gold, 4) if all_gold else 0.0,
        "exact_span_f1": _f1(exact_matches, all_predicted, all_gold),
        "type_accuracy_on_matched_spans": round(typed_matches / exact_matches, 4) if exact_matches else None,
        "review_accuracy_on_matched_spans": round(review_matches / exact_matches, 4) if exact_matches else None,
        "mean_evidence_f1_on_matched_spans": round(mean(evidence_f1_values), 4)
        if evidence_f1_values
        else None,
        "artifact_results": rows,
        "statistical_unit": "source artifact",
    }


def _validate_labels(
    artifact_id: str,
    labels: list[ClaimLabel] | list[AdjudicatedClaim],
    raw_text: str | None,
    errors: list[str],
    *,
    label_source: str,
) -> None:
    seen: set[tuple[int, int]] = set()
    for label in labels:
        span = _span(label)
        if label.span_end <= label.span_start:
            errors.append(f"Artifact {artifact_id} {label_source} has a non-positive span {span}.")
        if span in seen:
            errors.append(f"Artifact {artifact_id} {label_source} duplicates span {span}.")
        seen.add(span)
        if raw_text is not None:
            if label.span_end > len(raw_text):
                errors.append(f"Artifact {artifact_id} {label_source} span {span} exceeds raw text length.")
            elif raw_text[label.span_start : label.span_end] != label.text:
                errors.append(f"Artifact {artifact_id} {label_source} text does not match raw span {span}.")


def _artifact_agreement(
    artifact_id: str,
    first: AnnotationSet,
    second: AnnotationSet,
) -> dict[str, Any]:
    first_by_span = {_span(label): label for label in first.labels}
    second_by_span = {_span(label): label for label in second.labels}
    matches = sorted(set(first_by_span).intersection(second_by_span))
    types_a = [first_by_span[span].claim_type for span in matches]
    types_b = [second_by_span[span].claim_type for span in matches]
    review_a = [str(first_by_span[span].requires_review) for span in matches]
    review_b = [str(second_by_span[span].requires_review) for span in matches]
    evidence_f1 = [
        _set_f1(
            set(first_by_span[span].evidence_requirements),
            set(second_by_span[span].evidence_requirements),
        )
        for span in matches
    ]
    return {
        "artifact_id": artifact_id,
        "annotators": [first.annotator_id, second.annotator_id],
        "first_claims": len(first.labels),
        "second_claims": len(second.labels),
        "exact_span_matches": len(matches),
        "exact_span_f1": _f1(len(matches), len(first.labels), len(second.labels)),
        "claim_type_kappa_on_matched_spans": _cohen_kappa(types_a, types_b),
        "review_kappa_on_matched_spans": _cohen_kappa(review_a, review_b),
        "mean_evidence_f1_on_matched_spans": round(mean(evidence_f1), 4) if evidence_f1 else None,
    }


def _aggregate_agreement(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if not rows:
        return {
            "artifact_pairs": 0,
            "micro_exact_span_f1": None,
            "mean_claim_type_kappa_on_matched_spans": None,
            "mean_review_kappa_on_matched_spans": None,
            "mean_evidence_f1_on_matched_spans": None,
        }
    matches = sum(int(row["exact_span_matches"]) for row in rows)
    first_total = sum(int(row["first_claims"]) for row in rows)
    second_total = sum(int(row["second_claims"]) for row in rows)
    return {
        "artifact_pairs": len(rows),
        "micro_exact_span_f1": _f1(matches, first_total, second_total),
        "mean_claim_type_kappa_on_matched_spans": _mean_present(
            row["claim_type_kappa_on_matched_spans"] for row in rows
        ),
        "mean_review_kappa_on_matched_spans": _mean_present(
            row["review_kappa_on_matched_spans"] for row in rows
        ),
        "mean_evidence_f1_on_matched_spans": _mean_present(
            row["mean_evidence_f1_on_matched_spans"] for row in rows
        ),
    }


def _split_summary(artifacts: list[ClaimArtifact]) -> dict[str, Any]:
    return {
        split: {
            "artifacts": sum(1 for artifact in artifacts if artifact.split == split),
            "source_groups": len({artifact.source_group_id for artifact in artifacts if artifact.split == split}),
            "author_groups": len({artifact.author_group_id for artifact in artifacts if artifact.split == split}),
            "adjudicated_claims": sum(
                len(artifact.adjudicated_labels) for artifact in artifacts if artifact.split == split
            ),
        }
        for split in Split.__args__
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


def _f1(matches: int, first_total: int, second_total: int) -> float:
    if first_total + second_total == 0:
        return 1.0
    return round((2 * matches) / (first_total + second_total), 4)


def _set_f1(first: set[str], second: set[str]) -> float:
    return _f1(len(first.intersection(second)), len(first), len(second))


def _mean_present(values) -> float | None:
    present = [float(value) for value in values if value is not None]
    return round(mean(present), 4) if present else None


def _span(label: ClaimLabel) -> tuple[int, int]:
    return label.span_start, label.span_end


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
    parser = argparse.ArgumentParser(description="Validate or score independent claim annotations.")
    parser.add_argument("manifest")
    parser.add_argument("--predictions", default=None)
    parser.add_argument("--split", choices=list(Split.__args__), default="test")
    parser.add_argument("--allow-synthetic", action="store_true")
    parser.add_argument("--min-artifacts", type=int, default=120)
    parser.add_argument("--min-claims", type=int, default=600)
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    if args.predictions:
        result = score_claim_predictions(args.manifest, args.predictions, split=args.split)
    else:
        result = validate_claim_annotation_study(
            args.manifest,
            require_independent=not args.allow_synthetic,
            strict_min_artifacts=max(args.min_artifacts, 1),
            strict_min_adjudicated_claims=max(args.min_claims, 1),
        )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output:
        Path(args.output).write_text(rendered, encoding="utf-8")
    print(rendered)
    if not args.predictions and not result["structurally_admissible"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
