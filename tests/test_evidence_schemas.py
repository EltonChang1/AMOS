from __future__ import annotations

import json

from amos.evaluation.evidence_schemas import write_evidence_schemas


def test_evidence_schema_bundle_is_versioned_and_hashed(tmp_path) -> None:
    result = write_evidence_schemas(tmp_path / "schemas")
    assert result["schema_bundle_version"] == 1
    assert result["schema_count"] == 5
    assert all(len(item["sha256"]) == 64 for item in result["files"])
    task_schema = json.loads((tmp_path / "schemas" / "independent_task_study.schema.json").read_text())
    assert task_schema["title"] == "IndependentTaskStudy"
    assert "tasks" in task_schema["properties"]
    assert (tmp_path / "schemas" / "schema_manifest.json").exists()
