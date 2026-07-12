from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from amos.config import settings
from amos.evaluation.scenario_packs import READINESS_KEYS, load_scenario_packs
from amos.memory.store import MemoryStore
from amos.scenarios.fixtures import SUPPORTED_RUNTIME_FIXTURES, analytics_table_counts, seed_runtime_fixture


@dataclass(frozen=True)
class SettingsSnapshot:
    root: Path
    memory_db: Path
    analytics_db: Path
    artifact_dir: Path
    rotate_analytics_db_on_seed: bool


def load_scenario_pack(
    scenario_id: str,
    *,
    scenarios_dir: str | Path = "scenarios",
    run_dir: str | Path | None = None,
    write_artifacts: bool = True,
) -> dict[str, Any]:
    packs = {pack["pack_id"]: pack for pack in load_scenario_packs(scenarios_dir)}
    if scenario_id not in packs:
        raise ValueError(f"Unknown scenario pack: {scenario_id}")
    pack = packs[scenario_id]
    root = Path(run_dir or settings.artifact_dir / "scenario_loads" / scenario_id).resolve()
    bundle = _bundle_payload(pack)
    fixture_files: dict[str, str] = {}
    if write_artifacts:
        root.mkdir(parents=True, exist_ok=True)
        fixture_files = _write_bundle_files(root, bundle)

    materialized = _materialize_runtime_fixture(pack, root) if _should_seed_runtime_fixture(pack) else _manifest_only_fixture(pack)
    report = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "scenario_id": scenario_id,
        "version": pack["version"],
        "domain": pack["domain"],
        "status": _load_status(pack, materialized),
        "run_dir": str(root),
        "fixture_files": fixture_files,
        "task_count": len(pack["tasks"]),
        "expected_evidence_count": sum(len(task.get("expected_evidence", [])) for task in pack["tasks"]),
        "security_case_count": len(pack.get("security_cases", [])),
        "readiness": {key: pack["readiness"].get(key, "missing") for key in READINESS_KEYS},
        "readiness_gaps": _readiness_gaps(pack),
        "materialized_runtime": materialized,
        "paper_claim_boundary": _claim_boundary(pack, materialized),
    }
    if write_artifacts:
        (root / "load_report.json").write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")
        (root / "load_summary.md").write_text(_render_load_summary(report), encoding="utf-8")
    return report


def load_all_scenario_packs(
    *,
    scenarios_dir: str | Path = "scenarios",
    run_dir: str | Path | None = None,
    write_artifacts: bool = True,
) -> dict[str, Any]:
    root = Path(run_dir or settings.artifact_dir / "evaluation" / "scenario_loads").resolve()
    packs = load_scenario_packs(scenarios_dir)
    reports = [
        load_scenario_pack(
            pack["pack_id"],
            scenarios_dir=scenarios_dir,
            run_dir=root / pack["pack_id"],
            write_artifacts=write_artifacts,
        )
        for pack in packs
    ]
    aggregate = {
        "pack_count": len(reports),
        "runtime_seeded_count": sum(1 for report in reports if report["materialized_runtime"]["runtime_seeded"]),
        "manifest_only_count": sum(1 for report in reports if not report["materialized_runtime"]["runtime_seeded"]),
        "task_count": sum(report["task_count"] for report in reports),
        "expected_evidence_count": sum(report["expected_evidence_count"] for report in reports),
        "security_case_count": sum(report["security_case_count"] for report in reports),
    }
    result = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "scenarios_dir": str(Path(scenarios_dir).resolve()),
        "run_dir": str(root),
        "aggregate": aggregate,
        "reports": reports,
        "paper_claim_boundary": _all_claim_boundary(aggregate),
    }
    if write_artifacts:
        root.mkdir(parents=True, exist_ok=True)
        (root / "scenario_load_report.json").write_text(
            json.dumps(result, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        (root / "scenario_load_summary.md").write_text(_render_all_summary(result), encoding="utf-8")
    return result


def _bundle_payload(pack: dict[str, Any]) -> dict[str, Any]:
    assets = pack["assets"]
    return {
        "manifest": pack,
        "tasks": pack["tasks"],
        "expected": [
            {
                "task_id": task["task_id"],
                "expected_evidence": task.get("expected_evidence", []),
                "assertions": task.get("assertions", []),
                "perturbations": task.get("perturbations", []),
            }
            for task in pack["tasks"]
        ],
        "assets": assets,
        "risk_coverage": pack["risk_coverage"],
        "security_cases": pack.get("security_cases", []),
    }


def _write_bundle_files(root: Path, bundle: dict[str, Any]) -> dict[str, str]:
    paths = {
        "manifest": root / "scenario_manifest.json",
        "tasks": root / "tasks.json",
        "expected": root / "expected.json",
        "risk_coverage": root / "risk_coverage.json",
        "security_cases": root / "security_cases.json",
        "memory_objects": root / "memory" / "memory_objects.json",
        "data_tables": root / "data" / "data_tables.json",
        "policies": root / "policies" / "policies.json",
        "docs": root / "docs" / "docs.json",
    }
    for path in paths.values():
        path.parent.mkdir(parents=True, exist_ok=True)
    paths["manifest"].write_text(json.dumps(bundle["manifest"], indent=2, sort_keys=True), encoding="utf-8")
    paths["tasks"].write_text(json.dumps(bundle["tasks"], indent=2, sort_keys=True), encoding="utf-8")
    paths["expected"].write_text(json.dumps(bundle["expected"], indent=2, sort_keys=True), encoding="utf-8")
    paths["risk_coverage"].write_text(json.dumps(bundle["risk_coverage"], indent=2, sort_keys=True), encoding="utf-8")
    paths["security_cases"].write_text(json.dumps(bundle["security_cases"], indent=2, sort_keys=True), encoding="utf-8")
    paths["memory_objects"].write_text(
        json.dumps(bundle["assets"].get("memory_objects", []), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    paths["data_tables"].write_text(
        json.dumps(bundle["assets"].get("data_tables", []), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    paths["policies"].write_text(
        json.dumps(bundle["assets"].get("policies", []), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    paths["docs"].write_text(
        json.dumps(bundle["assets"].get("docs", []), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    return {name: str(path) for name, path in paths.items()}


def _should_seed_runtime_fixture(pack: dict[str, Any]) -> bool:
    readiness = pack["readiness"]
    return (
        pack["pack_id"] in SUPPORTED_RUNTIME_FIXTURES
        and readiness.get("data_seed") == "ready"
        and readiness.get("memory_seed") == "ready"
        and readiness.get("duckdb_seed") == "ready"
    )


def _materialize_runtime_fixture(pack: dict[str, Any], root: Path) -> dict[str, Any]:
    snapshot = _snapshot_settings()
    try:
        settings.use_run_dir(root, rotate_analytics_db_on_seed=False)
        if pack["pack_id"] != "payment_failure":
            settings.use_paths(analytics_db=root / "data" / "synthetic" / f"{pack['pack_id']}.duckdb")
        settings.ensure_dirs()
        loader_mode = seed_runtime_fixture(pack["pack_id"])
        store = MemoryStore(settings.memory_db)
        memory_ids = sorted(item.id for item in store.list_memory())
        table_counts = analytics_table_counts()
        return {
            "runtime_seeded": True,
            "loader_mode": loader_mode,
            "memory_db": str(settings.memory_db),
            "analytics_db": str(settings.analytics_db),
            "artifact_dir": str(settings.artifact_dir),
            "memory_object_count": len(memory_ids),
            "memory_ids": memory_ids,
            "analytics_db_exists": settings.analytics_db.exists(),
            "analytics_table_counts": table_counts,
            "supported_eval_command": pack.get("supported_eval_commands", [""])[0] if pack.get("supported_eval_commands") else "",
        }
    finally:
        _restore_settings(snapshot)


def _manifest_only_fixture(pack: dict[str, Any]) -> dict[str, Any]:
    return {
        "runtime_seeded": False,
        "loader_mode": "manifest_specification",
        "memory_db": "",
        "analytics_db": "",
        "artifact_dir": "",
        "memory_object_count": 0,
        "memory_ids": [],
        "analytics_db_exists": False,
        "analytics_table_counts": {},
        "supported_eval_command": "",
        "next_steps": [
            "implement synthetic data seeder",
            "implement memory fixture seeder",
            "wire live-agent and product_eval adapters",
        ],
    }


def _load_status(pack: dict[str, Any], materialized: dict[str, Any]) -> str:
    if materialized["runtime_seeded"]:
        return "runtime_seeded"
    if pack["status"] == "scenario_manifest_only":
        return "manifest_specification"
    return "partial"


def _readiness_gaps(pack: dict[str, Any]) -> list[str]:
    return [
        f"{key}: {pack['readiness'].get(key, 'missing')}"
        for key in READINESS_KEYS
        if pack["readiness"].get(key) != "ready"
    ]


def _claim_boundary(pack: dict[str, Any], materialized: dict[str, Any]) -> list[str]:
    if materialized["runtime_seeded"]:
        return [
            f"{pack['pack_id']} has a materialized dev runtime fixture with seeded memory and DuckDB data.",
            "This proves loadability for this dev fixture, not provider-backed robustness or product-eval generality.",
        ]
    return [
        f"{pack['pack_id']} has an inspectable scenario specification bundle.",
        "This does not prove executable product_eval behavior until data, memory, and adapter seeders are implemented.",
    ]


def _all_claim_boundary(aggregate: dict[str, Any]) -> list[str]:
    claims = [
        f"Scenario loader materialized {aggregate['pack_count']} scenario bundles.",
        f"{aggregate['runtime_seeded_count']}/{aggregate['pack_count']} bundles include seeded runtime data stores.",
    ]
    if aggregate["manifest_only_count"]:
        claims.append("Manifest-only bundles are inspectable fixture specifications, not executable product_eval adapters.")
    else:
        claims.append("All loaded bundles include seeded runtime data stores; this proves loadability, not provider-backed robustness.")
    return claims


def _render_load_summary(report: dict[str, Any]) -> str:
    lines = [
        f"# Scenario Load: {report['scenario_id']}",
        "",
        f"Generated: {report['generated_at']}",
        f"Status: {report['status']}",
        f"Domain: {report['domain']}",
        f"Tasks: {report['task_count']}",
        f"Expected evidence hooks: {report['expected_evidence_count']}",
        f"Security cases: {report['security_case_count']}",
        "",
        "## Runtime",
        "",
        f"- Runtime seeded: {report['materialized_runtime']['runtime_seeded']}",
        f"- Loader mode: {report['materialized_runtime']['loader_mode']}",
        f"- Memory objects: {report['materialized_runtime']['memory_object_count']}",
        "",
        "## Claim Boundary",
        "",
    ]
    lines.extend(f"- {claim}" for claim in report["paper_claim_boundary"])
    lines.append("")
    return "\n".join(lines)


def _render_all_summary(result: dict[str, Any]) -> str:
    aggregate = result["aggregate"]
    lines = [
        "# AMOS Scenario Loader Report",
        "",
        f"Generated: {result['generated_at']}",
        f"Scenario bundles: {aggregate['pack_count']}",
        f"Runtime-seeded bundles: {aggregate['runtime_seeded_count']}/{aggregate['pack_count']}",
        f"Manifest-only bundles: {aggregate['manifest_only_count']}",
        "",
        "| Scenario | Status | Tasks | Expected Evidence | Runtime Seeded |",
        "| --- | --- | ---: | ---: | --- |",
    ]
    for report in result["reports"]:
        lines.append(
            f"| {report['scenario_id']} | {report['status']} | {report['task_count']} | "
            f"{report['expected_evidence_count']} | {report['materialized_runtime']['runtime_seeded']} |"
        )
    lines.extend(["", "## Claim Boundary", ""])
    lines.extend(f"- {claim}" for claim in result["paper_claim_boundary"])
    lines.append("")
    return "\n".join(lines)


def _snapshot_settings() -> SettingsSnapshot:
    return SettingsSnapshot(
        root=settings.root,
        memory_db=settings.memory_db,
        analytics_db=settings.analytics_db,
        artifact_dir=settings.artifact_dir,
        rotate_analytics_db_on_seed=settings.rotate_analytics_db_on_seed,
    )


def _restore_settings(snapshot: SettingsSnapshot) -> None:
    settings.root = snapshot.root
    settings.memory_db = snapshot.memory_db
    settings.analytics_db = snapshot.analytics_db
    settings.artifact_dir = snapshot.artifact_dir
    settings.rotate_analytics_db_on_seed = snapshot.rotate_analytics_db_on_seed


def main() -> None:
    parser = argparse.ArgumentParser(description="Load AMOS scenario packs into a run directory.")
    parser.add_argument("scenario_id", nargs="?", help="Scenario pack id, for example payment_failure.")
    parser.add_argument("--all", action="store_true", help="Load every scenario pack under --scenarios-dir.")
    parser.add_argument("--scenarios-dir", default="scenarios")
    parser.add_argument("--run-dir", default=None)
    args = parser.parse_args()
    if args.all:
        result = load_all_scenario_packs(scenarios_dir=args.scenarios_dir, run_dir=args.run_dir)
    else:
        if not args.scenario_id:
            parser.error("scenario_id is required unless --all is set")
        result = load_scenario_pack(args.scenario_id, scenarios_dir=args.scenarios_dir, run_dir=args.run_dir)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
