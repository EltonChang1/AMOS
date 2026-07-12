from __future__ import annotations

import json
from pathlib import Path

from amos.config import settings
from amos.scenarios.load import load_all_scenario_packs, load_scenario_pack


def test_load_payment_scenario_materializes_runtime_fixture(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "payment_failure" / "scenario.json", "payment_failure", "ready")
    original_memory_db = settings.memory_db

    report = load_scenario_pack(
        "payment_failure",
        scenarios_dir=scenarios_dir,
        run_dir=tmp_path / "payment_run",
    )

    assert report["status"] == "runtime_seeded"
    assert report["materialized_runtime"]["runtime_seeded"] is True
    assert report["materialized_runtime"]["memory_object_count"] > 0
    assert Path(report["materialized_runtime"]["memory_db"]).exists()
    assert Path(report["materialized_runtime"]["analytics_db"]).exists()
    assert Path(report["fixture_files"]["tasks"]).exists()
    assert settings.memory_db == original_memory_db


def test_load_manifest_only_scenario_writes_inspectable_bundle(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "subscription_churn" / "scenario.json", "subscription_churn", "not_started")

    report = load_scenario_pack(
        "subscription_churn",
        scenarios_dir=scenarios_dir,
        run_dir=tmp_path / "subscription_run",
    )

    assert report["status"] == "manifest_specification"
    assert report["materialized_runtime"]["runtime_seeded"] is False
    assert report["readiness_gaps"]
    assert Path(report["fixture_files"]["expected"]).exists()
    assert json.loads(Path(report["fixture_files"]["memory_objects"]).read_text(encoding="utf-8"))


def test_load_subscription_scenario_materializes_runtime_fixture_when_ready(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "subscription_churn" / "scenario.json", "subscription_churn", "ready")

    report = load_scenario_pack(
        "subscription_churn",
        scenarios_dir=scenarios_dir,
        run_dir=tmp_path / "subscription_runtime",
    )

    assert report["status"] == "runtime_seeded"
    runtime = report["materialized_runtime"]
    assert runtime["loader_mode"] == "seeded_subscription_churn_fixture"
    assert runtime["runtime_seeded"] is True
    assert runtime["memory_object_count"] >= 10
    assert runtime["analytics_table_counts"]["subscriptions"] == 240
    assert runtime["analytics_table_counts"]["support_contact_rollups"] > 0
    assert Path(runtime["analytics_db"]).exists()


def test_load_warehouse_scenario_materializes_runtime_fixture_when_ready(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "warehouse_quality" / "scenario.json", "warehouse_quality", "ready")

    report = load_scenario_pack(
        "warehouse_quality",
        scenarios_dir=scenarios_dir,
        run_dir=tmp_path / "warehouse_runtime",
    )

    assert report["status"] == "runtime_seeded"
    runtime = report["materialized_runtime"]
    assert runtime["loader_mode"] == "seeded_warehouse_quality_fixture"
    assert runtime["runtime_seeded"] is True
    assert runtime["memory_object_count"] >= 10
    assert runtime["analytics_table_counts"]["pick_pack_events"] == 480
    assert runtime["analytics_table_counts"]["inventory_events"] == 320
    assert runtime["analytics_table_counts"]["vendor_quality_rollups"] > 0
    assert Path(runtime["analytics_db"]).exists()


def test_load_all_scenarios_writes_aggregate_report(tmp_path: Path) -> None:
    scenarios_dir = tmp_path / "scenarios"
    _write_pack(scenarios_dir / "payment_failure" / "scenario.json", "payment_failure", "ready")
    _write_pack(scenarios_dir / "warehouse_quality" / "scenario.json", "warehouse_quality", "not_started")

    result = load_all_scenario_packs(scenarios_dir=scenarios_dir, run_dir=tmp_path / "loads")

    assert result["aggregate"]["pack_count"] == 2
    assert result["aggregate"]["runtime_seeded_count"] == 1
    assert result["aggregate"]["manifest_only_count"] == 1
    assert (tmp_path / "loads" / "scenario_load_report.json").exists()
    assert (tmp_path / "loads" / "payment_failure" / "load_report.json").exists()


def _write_pack(path: Path, pack_id: str, readiness: str) -> None:
    path.parent.mkdir(parents=True)
    payload = {
        "pack_id": pack_id,
        "version": "test",
        "domain": f"{pack_id} domain",
        "status": "executable_product_eval" if readiness == "ready" else "scenario_manifest_only",
        "description": "Synthetic scenario pack for loader tests.",
        "supported_eval_commands": ["python3 -m amos.evaluation.product_eval --scenario payment_failure"] if readiness == "ready" else [],
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
