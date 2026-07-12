"""Write versioned JSON Schemas for external evidence manifests and predictions."""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.evaluation.claim_annotation_evidence import ClaimAnnotationStudy, ClaimPredictionManifest
from amos.evaluation.external_product_evidence import ExternalProductStudy
from amos.evaluation.independent_task_evidence import IndependentTaskStudy, TaskPredictionManifest


SCHEMA_MODELS = {
    "claim_annotation_study.schema.json": ClaimAnnotationStudy,
    "claim_predictions.schema.json": ClaimPredictionManifest,
    "external_product_study.schema.json": ExternalProductStudy,
    "independent_task_study.schema.json": IndependentTaskStudy,
    "task_predictions.schema.json": TaskPredictionManifest,
}


def write_evidence_schemas(output_dir: str | Path) -> dict[str, Any]:
    output = Path(output_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    files = []
    for filename, model in sorted(SCHEMA_MODELS.items()):
        path = output / filename
        rendered = json.dumps(model.model_json_schema(), indent=2, sort_keys=True) + "\n"
        path.write_text(rendered, encoding="utf-8")
        files.append(
            {
                "path": str(path),
                "bytes": len(rendered.encode("utf-8")),
                "sha256": hashlib.sha256(rendered.encode("utf-8")).hexdigest(),
            }
        )
    manifest = {
        "schema_bundle_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "schema_count": len(files),
        "files": files,
        "evidence_boundary": (
            "Schemas define admissible inputs and scoring contracts. Their presence does not prove that "
            "independent participants, real artifacts, live models, or deployed products completed a study."
        ),
    }
    (output / "schema_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description="Write AMOS independent-evidence JSON Schemas.")
    parser.add_argument("--output-dir", default="artifacts/evaluation/evidence_schemas")
    args = parser.parse_args()
    print(json.dumps(write_evidence_schemas(args.output_dir), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
