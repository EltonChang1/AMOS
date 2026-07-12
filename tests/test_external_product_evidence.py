from __future__ import annotations

import hashlib
import json
from pathlib import Path

from amos.evaluation.external_product_evidence import validate_external_product_study


def _manifest(tmp_path: Path, deployment_mode: str = "external_saas") -> Path:
    raw = tmp_path / "raw" / "run.json"
    raw.parent.mkdir(parents=True)
    raw.write_text('{"response":"ok"}', encoding="utf-8")
    raw_hash = hashlib.sha256(raw.read_bytes()).hexdigest()
    payload = {
        "study_id": "external-study-001",
        "protocol_version": 1,
        "task_label_source": "independent analyst panel",
        "runs": [
            {
                "run_id": "run-001",
                "task_id": "task-001",
                "scenario": "payment_failure",
                "product_name": "Example Product",
                "product_version": "2026.07",
                "deployment_mode": deployment_mode,
                "system_category": "catalog_lineage",
                "user_id": "analyst-001",
                "permissions": ["analytics", "payments"],
                "configuration_sha256": "a" * 64,
                "raw_evidence_path": "raw/run.json",
                "raw_evidence_sha256": raw_hash,
                "latency_seconds": 1.2,
                "token_usage": {"total_tokens": 100},
                "cost_usd": 0.01,
                "metrics": {
                    "task_correctness": True,
                    "sql_validity": True,
                    "metric_correctness": True,
                    "schema_correctness": True,
                    "permission_safety": True,
                    "provenance_coverage": 0.5,
                    "replay_success": False,
                    "review_obligation_recall": 1.0
                }
            }
        ]
    }
    manifest = tmp_path / "manifest.json"
    manifest.write_text(json.dumps(payload), encoding="utf-8")
    return manifest


def test_external_product_manifest_validates_hashes_and_axes(tmp_path: Path) -> None:
    result = validate_external_product_study(_manifest(tmp_path))
    assert result["admissible"] is True
    assert result["aggregate"]["Example Product"]["independent_task_units"] == 1
    assert result["aggregate"]["Example Product"]["metric_means"]["task_correctness"] == 1.0


def test_local_adapter_cannot_be_relabelled_external(tmp_path: Path) -> None:
    result = validate_external_product_study(_manifest(tmp_path, "local_export_shaped_adapter"))
    assert result["admissible"] is False
    assert any("not external product evidence" in error for error in result["errors"])
