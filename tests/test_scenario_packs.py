from __future__ import annotations

import csv
import json
from pathlib import Path

import pytest

from amos.evaluation.scenario_packs import evaluate_scenario_packs, load_scenario_packs, run_generated_scenario_tasks


def test_scenario_pack_evaluator_writes_readiness_artifacts(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "ready" / "scenario.json", "ready_pack", "ready")
    _write_pack(scenarios_dir / "scaffold" / "scenario.json", "scaffold_pack", "not_started")

    results = evaluate_scenario_packs(scenarios_dir, tmp_path / "out")

    output_dir = Path(results["output_dir"])
    assert (output_dir / "scenario_pack_report.json").exists()
    assert (output_dir / "scenario_pack_summary.md").exists()
    assert (output_dir / "scenario_pack_coverage.csv").exists()
    assert results["aggregate"]["pack_count"] == 2
    assert results["aggregate"]["executable_product_eval_count"] == 1
    assert results["aggregate"]["total_tasks"] == 2
    assert results["packs"][0]["manifest_completeness_score"] == 1.0
    assert "Paper Claim Boundary" in (output_dir / "scenario_pack_summary.md").read_text(encoding="utf-8")

    with (output_dir / "scenario_pack_coverage.csv").open(encoding="utf-8", newline="") as handle:
        rows = list(csv.DictReader(handle))
    assert len(rows) == 2
    assert rows[0]["risk_metric_correctness"] == "True"


def test_scenario_pack_loader_validates_required_fields(tmp_path: Path) -> None:
    bad_path = tmp_path / "scenarios" / "bad" / "scenario.json"
    bad_path.parent.mkdir(parents=True)
    bad_path.write_text(json.dumps({"pack_id": "bad"}), encoding="utf-8")

    with pytest.raises(ValueError, match="missing required fields"):
        load_scenario_packs(tmp_path / "scenarios")


def test_generated_scenario_tasks_write_raw_seeded_records(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "ready" / "scenario.json", "ready_pack", "ready")
    _write_pack(scenarios_dir / "scaffold" / "scenario.json", "scaffold_pack", "not_started")

    results = run_generated_scenario_tasks(
        scenarios_dir,
        variants=8,
        seed=7,
        output_dir=tmp_path / "out",
    )

    output_dir = Path(results["output_dir"])
    assert (output_dir / "generated_tasks.json").exists()
    assert (output_dir / "generated_tasks_summary.md").exists()
    assert (output_dir / "generated_tasks.csv").exists()
    assert results["aggregate"]["runs"] == 8
    assert results["aggregate"]["manifest_contract_passed"] == 8
    assert results["aggregate"]["product_eval_executable_runs"] == 4
    assert results["aggregate"]["manifest_only_runs"] == 4
    assert results["aggregate"]["raw_evidence_count"] == 8

    for record in results["records"]:
        assert Path(record["raw_path"]).exists()
        assert record["perturbations"]


def _write_pack(path: Path, pack_id: str, readiness: str) -> None:
    path.parent.mkdir(parents=True)
    payload = {
        "pack_id": pack_id,
        "version": "test",
        "domain": f"{pack_id} domain",
        "status": "test",
        "description": "Synthetic scenario pack for tests.",
        "supported_eval_commands": [],
        "assets": {
            "data_tables": ["events"],
            "memory_objects": ["memory_metric"],
            "policies": ["policy"],
            "docs": ["doc"],
        },
        "tasks": [
            {
                "task_id": "task",
                "family": "end_to_end",
                "request": "Explain the change.",
                "permissions": ["analytics"],
                "expected_evidence": ["memory_metric"],
                "assertions": ["cite evidence"],
                "perturbations": ["direct_wording"],
            }
        ],
        "security_cases": ["restricted memory"],
        "risk_coverage": {
            "metric_correctness": True,
            "schema_drift": True,
            "temporal_freshness": True,
            "permission_filtering": True,
            "provenance": True,
            "replay": True,
            "prompt_injection": True,
            "memory_poisoning": True,
            "human_review_boundary": True,
        },
        "readiness": {
            "data_seed": readiness,
            "memory_seed": readiness,
            "duckdb_seed": readiness,
            "product_eval_adapter": readiness,
            "baseline_adapter": readiness,
            "live_agent_adapter": readiness,
        },
        "known_gaps": [],
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
