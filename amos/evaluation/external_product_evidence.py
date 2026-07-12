from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path
from statistics import mean
from typing import Any, Literal

from pydantic import BaseModel, Field


class ExternalMetricAxes(BaseModel):
    task_correctness: bool
    sql_validity: bool
    metric_correctness: bool
    schema_correctness: bool
    permission_safety: bool
    provenance_coverage: float = Field(ge=0.0, le=1.0)
    replay_success: bool
    review_obligation_recall: float = Field(ge=0.0, le=1.0)


class ExternalProductRun(BaseModel):
    run_id: str
    task_id: str
    scenario: str
    product_name: str
    product_version: str
    deployment_mode: Literal["external_saas", "self_hosted_product", "local_export_shaped_adapter"]
    system_category: Literal["rag", "semantic_layer", "catalog_lineage", "agent", "amos"]
    user_id: str
    permissions: list[str]
    configuration_sha256: str
    raw_evidence_path: str
    raw_evidence_sha256: str
    latency_seconds: float = Field(ge=0.0)
    token_usage: dict[str, int] = Field(default_factory=dict)
    cost_usd: float | None = Field(default=None, ge=0.0)
    metrics: ExternalMetricAxes


class ExternalProductStudy(BaseModel):
    study_id: str
    protocol_version: int = 1
    task_label_source: str
    runs: list[ExternalProductRun]


def validate_external_product_study(
    manifest_path: str | Path,
    *,
    require_external_deployment: bool = True,
) -> dict[str, Any]:
    """Validate raw evidence and summarize external-system runs.

    This function never upgrades fixture adapters into external evidence. When
    ``require_external_deployment`` is true, any local export-shaped run makes
    the study inadmissible for a hosted/self-hosted product comparison claim.
    """

    manifest = Path(manifest_path).resolve()
    study = ExternalProductStudy.model_validate_json(manifest.read_text(encoding="utf-8"))
    root = manifest.parent
    errors: list[str] = []
    seen_run_ids: set[str] = set()
    seen_product_tasks: set[tuple[str, str]] = set()
    rows: list[dict[str, Any]] = []

    for run in study.runs:
        if run.run_id in seen_run_ids:
            errors.append(f"Duplicate run_id: {run.run_id}")
        seen_run_ids.add(run.run_id)
        product_task = (run.product_name, run.task_id)
        if product_task in seen_product_tasks:
            errors.append(
                f"Duplicate product/task statistical unit: {run.product_name}/{run.task_id}; "
                "record repeats separately rather than treating them as independent tasks."
            )
        seen_product_tasks.add(product_task)
        if require_external_deployment and run.deployment_mode == "local_export_shaped_adapter":
            errors.append(f"Run {run.run_id} is a local export-shaped adapter, not external product evidence.")
        if len(run.configuration_sha256) != 64:
            errors.append(f"Run {run.run_id} has an invalid configuration_sha256.")
        raw_path = _safe_evidence_path(root, run.raw_evidence_path)
        if raw_path is None or not raw_path.exists():
            errors.append(f"Run {run.run_id} raw evidence is missing or escapes the manifest directory.")
            raw_exists = False
            observed_hash = None
        else:
            raw_exists = True
            observed_hash = _sha256(raw_path)
            if observed_hash != run.raw_evidence_sha256:
                errors.append(f"Run {run.run_id} raw evidence SHA-256 mismatch.")
        rows.append(
            {
                **run.model_dump(mode="json"),
                "raw_evidence_exists": raw_exists,
                "observed_raw_evidence_sha256": observed_hash,
            }
        )

    by_product: dict[str, list[ExternalProductRun]] = defaultdict(list)
    for run in study.runs:
        by_product[run.product_name].append(run)

    aggregate = {}
    for product, runs in sorted(by_product.items()):
        aggregate[product] = {
            "independent_task_units": len({run.task_id for run in runs}),
            "executions": len(runs),
            "deployment_modes": sorted({run.deployment_mode for run in runs}),
            "mean_latency_seconds": round(mean(run.latency_seconds for run in runs), 4),
            "metric_means": {
                field: round(mean(float(getattr(run.metrics, field)) for run in runs), 3)
                for field in ExternalMetricAxes.model_fields
            },
            "statistical_note": "Descriptive task-level means; no population interval is inferred by this validator.",
        }

    return {
        "study_id": study.study_id,
        "protocol_version": study.protocol_version,
        "task_label_source": study.task_label_source,
        "manifest": str(manifest),
        "require_external_deployment": require_external_deployment,
        "admissible": not errors,
        "errors": errors,
        "aggregate": aggregate,
        "runs": rows,
    }


def _safe_evidence_path(root: Path, relative: str) -> Path | None:
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
    parser = argparse.ArgumentParser(description="Validate an AMOS external-product evidence manifest.")
    parser.add_argument("manifest")
    parser.add_argument("--allow-local-adapters", action="store_true")
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    result = validate_external_product_study(
        args.manifest,
        require_external_deployment=not args.allow_local_adapters,
    )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.output:
        Path(args.output).write_text(rendered, encoding="utf-8")
    print(rendered)
    if not result["admissible"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
