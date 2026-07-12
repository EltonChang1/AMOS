from __future__ import annotations

import json

from amos.evaluation.systems_scale import run_systems_scale_experiment


def test_systems_scale_experiment_preserves_governance_and_writes_artifacts(tmp_path) -> None:
    output_dir = tmp_path / "systems_scale"
    result = run_systems_scale_experiment(
        memory_items=1001,
        readers=2,
        mixed_writes=4,
        provenance_edges=1000,
        retrieval_repeats=3,
        output_dir=output_dir,
    )

    assert result["status"] == "completed"
    assert result["memory_scale"]["index_synchronized"] is True
    assert result["serial_retrieval"]["passed"] == 3
    assert result["serial_retrieval"]["target_rank"] == 1
    assert result["concurrent_reads"]["errors"] == []
    assert result["update_consistency"]["permission_revocation"]["passed"] is True
    assert result["update_consistency"]["metric_supersession"]["passed"] is True
    assert result["mixed_read_write"]["errors"] == []
    assert result["provenance_growth"]["edges_inserted"] == 1000
    assert result["provenance_growth"]["target_query_returned"] == 10

    archived = json.loads((output_dir / "results.json").read_text(encoding="utf-8"))
    assert archived["schema_version"] == "amos.systems_scale.v1"
    assert (output_dir / "results.sha256").exists()
    assert (output_dir / "summary.md").exists()
