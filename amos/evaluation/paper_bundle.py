from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import platform
import subprocess
import sys
import time
import traceback
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

from amos.config import settings
from amos.evaluation.archive_live_pilot import archive_live_pilot
from amos.evaluation.archive_retrieval_comparison import archive_retrieval_comparisons
from amos.evaluation.benchmark import run_benchmark_suite
from amos.evaluation.evidence_schemas import write_evidence_schemas
from amos.evaluation.extended_experiments import run_extended_experiments
from amos.evaluation.paper_report import generate_paper_results_report
from amos.evaluation.product_eval import ABLATION_SYSTEMS, DEFAULT_SYSTEMS, run_product_eval
from amos.evaluation.scenario_packs import evaluate_scenario_packs, run_generated_scenario_tasks
from amos.evaluation.systems_scale import run_systems_scale_experiment
from amos.scenarios.load import load_all_scenario_packs


@dataclass(frozen=True)
class SettingsSnapshot:
    root: Path
    memory_db: Path
    analytics_db: Path
    artifact_dir: Path
    rotate_analytics_db_on_seed: bool


PAPER_SYSTEMS = [*DEFAULT_SYSTEMS, *ABLATION_SYSTEMS]


def run_paper_bundle(
    *,
    run_dir: str | Path,
    scenarios_dir: str | Path = "scenarios",
    variants: int = 12,
    samples: int = 3,
    generated_tasks: int = 120,
    variant_seed: int = 20260711,
    scale_items: int = 5000,
    concurrency: int = 8,
    benchmark_samples: int = 3,
    llm_samples: int = 1,
    claim_items: int = 1000,
    systems_scale_sizes: list[int] | None = None,
    systems_scale_readers: int = 8,
    systems_scale_writes: int = 64,
    live_pilot_dir: str | Path | None = None,
    retrieval_comparison_dirs: list[str | Path] | None = None,
    provider_mode: str = "auto",
    run_tests: bool = True,
) -> dict[str, Any]:
    """Regenerate the complete offline-capable paper artifact bundle.

    Provider mode ``auto`` records an offline-provider boundary when credentials
    are absent. Stage failures are preserved in the manifest and do not erase
    artifacts from stages that completed successfully.
    """
    started = time.perf_counter()
    root = Path(run_dir).resolve()
    root.mkdir(parents=True, exist_ok=True)
    scenarios_root = Path(scenarios_dir).resolve()
    snapshot = _snapshot_settings()
    stages: list[dict[str, Any]] = []
    config = {
        "run_dir": str(root),
        "scenarios_dir": str(scenarios_root),
        "variants": max(variants, 1),
        "samples": max(samples, 1),
        "generated_tasks": max(generated_tasks, 1),
        "variant_seed": variant_seed,
        "scale_items": max(scale_items, 0),
        "concurrency": max(concurrency, 1),
        "benchmark_samples": max(benchmark_samples, 1),
        "llm_samples": max(llm_samples, 1),
        "claim_items": max(claim_items, 4),
        "systems_scale_sizes": [max(size, 0) for size in (systems_scale_sizes or [])],
        "systems_scale_readers": max(systems_scale_readers, 1),
        "systems_scale_writes": max(systems_scale_writes, 0),
        "live_pilot_dir": str(Path(live_pilot_dir).resolve()) if live_pilot_dir else None,
        "retrieval_comparison_dirs": [
            str(Path(path).resolve()) for path in (retrieval_comparison_dirs or [])
        ],
        "provider_mode": provider_mode,
        "run_tests": run_tests,
        "systems": PAPER_SYSTEMS,
    }
    try:
        settings.use_run_dir(root)
        settings.ensure_dirs()
        evaluation_dir = settings.artifact_dir / "evaluation"
        logs_dir = evaluation_dir / "logs"
        logs_dir.mkdir(parents=True, exist_ok=True)
        (evaluation_dir / "environment.json").write_text(
            json.dumps(_environment_metadata(config), indent=2, sort_keys=True), encoding="utf-8"
        )

        _run_stage(
            stages,
            "source_manifest",
            logs_dir,
            lambda: _write_source_manifest(evaluation_dir),
        )

        if run_tests:
            _run_stage(stages, "unit_tests", logs_dir, lambda: _run_tests(logs_dir))
        else:
            stages.append({"name": "unit_tests", "status": "skipped", "reason": "--skip-tests"})

        _run_stage(
            stages,
            "scenario_loads",
            logs_dir,
            lambda: load_all_scenario_packs(
                scenarios_dir=scenarios_root,
                run_dir=evaluation_dir / "scenario_loads",
                write_artifacts=True,
            ),
        )
        _run_stage(
            stages,
            "evidence_schemas",
            logs_dir,
            lambda: write_evidence_schemas(evaluation_dir / "evidence_schemas"),
        )
        if live_pilot_dir:
            _run_stage(
                stages,
                "live_pilot_archive",
                logs_dir,
                lambda: archive_live_pilot(live_pilot_dir, evaluation_dir / "live_llm_pilot"),
            )
        if config["retrieval_comparison_dirs"]:
            _run_stage(
                stages,
                "retrieval_engine_comparison_archive",
                logs_dir,
                lambda: archive_retrieval_comparisons(
                    config["retrieval_comparison_dirs"],
                    evaluation_dir / "retrieval_engine_comparison",
                ),
            )
        _run_stage(
            stages,
            "scenario_packs",
            logs_dir,
            lambda: _run_scenario_pack_stage(
                scenarios_root,
                evaluation_dir / "scenario_packs",
                config["generated_tasks"],
                variant_seed,
            ),
        )

        for scenario in ["payment_failure", "subscription_churn", "warehouse_quality"]:
            _run_stage(
                stages,
                f"product_eval_{scenario}",
                logs_dir,
                lambda scenario=scenario: run_product_eval(
                    scenario=scenario,
                    variants=config["variants"],
                    samples=config["samples"],
                    systems=PAPER_SYSTEMS,
                    provider_mode=provider_mode,
                    variant_seed=variant_seed,
                    write_artifacts=True,
                ),
            )

        _reset_bundle_settings(root)
        _run_stage(
            stages,
            "benchmark_suite",
            logs_dir,
            lambda: _run_benchmark_stage(evaluation_dir, config["benchmark_samples"], config["scale_items"]),
        )
        _reset_bundle_settings(root)
        _run_stage(
            stages,
            "extended_experiments",
            logs_dir,
            lambda: run_extended_experiments(
                scale_items=config["scale_items"],
                concurrency=config["concurrency"],
                llm_samples=config["llm_samples"],
                claim_items=config["claim_items"],
                write_artifacts=True,
            ),
        )
        for size in config["systems_scale_sizes"]:
            label = _scale_label(size)
            _run_stage(
                stages,
                f"systems_scale_{label}",
                logs_dir,
                lambda size=size, label=label: run_systems_scale_experiment(
                    memory_items=size,
                    readers=config["systems_scale_readers"],
                    mixed_writes=config["systems_scale_writes"],
                    provenance_edges=size,
                    retrieval_repeats=20 if size >= 1_000_000 else 30,
                    output_dir=evaluation_dir / f"systems_scale_{label}",
                ),
            )
        _run_stage(
            stages,
            "paper_report",
            logs_dir,
            lambda: generate_paper_results_report(evaluation_dir),
        )

        _write_reproduction_record(root, config)
        artifact_manifest = _artifact_manifest(settings.artifact_dir)
        (evaluation_dir / "artifact_manifest.json").write_text(
            json.dumps(artifact_manifest, indent=2, sort_keys=True), encoding="utf-8"
        )
        status = "pass" if all(stage["status"] in {"pass", "skipped"} for stage in stages) else "fail"
        manifest = {
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "status": status,
            "duration_seconds": round(time.perf_counter() - started, 3),
            "config": config,
            "stages": stages,
            "artifact_count": artifact_manifest["artifact_count"],
            "artifact_bytes": artifact_manifest["total_bytes"],
            "paper_results": str(evaluation_dir / "PAPER_RESULTS.md"),
            "artifact_manifest": str(evaluation_dir / "artifact_manifest.json"),
        }
        rendered_manifest = json.dumps(manifest, indent=2, sort_keys=True) + "\n"
        (evaluation_dir / "bundle_manifest.json").write_text(rendered_manifest, encoding="utf-8")
        bundle_digest = hashlib.sha256(rendered_manifest.encode("utf-8")).hexdigest()
        (evaluation_dir / "bundle_manifest.sha256").write_text(
            f"{bundle_digest}  bundle_manifest.json\n",
            encoding="utf-8",
        )
        return manifest
    finally:
        _restore_settings(snapshot)


def _run_stage(
    stages: list[dict[str, Any]],
    name: str,
    logs_dir: Path,
    operation: Callable[[], Any],
) -> None:
    started = time.perf_counter()
    try:
        result = operation()
        summary = _stage_summary(result)
        stage = {
            "name": name,
            "status": "pass",
            "duration_seconds": round(time.perf_counter() - started, 3),
            "summary": summary,
        }
        (logs_dir / f"{name}.json").write_text(json.dumps(stage, indent=2, sort_keys=True), encoding="utf-8")
    except Exception as exc:
        stage = {
            "name": name,
            "status": "fail",
            "duration_seconds": round(time.perf_counter() - started, 3),
            "error": f"{type(exc).__name__}: {exc}",
        }
        (logs_dir / f"{name}.log").write_text(traceback.format_exc(), encoding="utf-8")
    stages.append(stage)


def _run_tests(logs_dir: Path) -> dict[str, Any]:
    project_root = Path(__file__).resolve().parents[2]
    completed = subprocess.run(
        [sys.executable, "-m", "pytest", "-q"],
        cwd=project_root,
        text=True,
        capture_output=True,
        check=False,
    )
    output = completed.stdout + completed.stderr
    (logs_dir / "pytest.log").write_text(output, encoding="utf-8")
    if completed.returncode != 0:
        raise RuntimeError(f"pytest exited with {completed.returncode}; see {logs_dir / 'pytest.log'}")
    return {"returncode": completed.returncode, "last_line": output.strip().splitlines()[-1] if output.strip() else ""}


def _run_scenario_pack_stage(
    scenarios_dir: Path,
    output_dir: Path,
    generated_tasks: int,
    seed: int,
) -> dict[str, Any]:
    pack_report = evaluate_scenario_packs(scenarios_dir, output_dir, write_artifacts=True)
    generated_report = run_generated_scenario_tasks(
        scenarios_dir,
        variants=generated_tasks,
        seed=seed,
        output_dir=output_dir,
        write_artifacts=True,
    )
    return {
        "pack_aggregate": pack_report["aggregate"],
        "generated_aggregate": generated_report["aggregate"],
    }


def _run_benchmark_stage(evaluation_dir: Path, samples: int, scale_items: int) -> dict[str, Any]:
    results = run_benchmark_suite(samples=samples, scale_items=scale_items)
    (evaluation_dir / "benchmark_suite.json").write_text(
        json.dumps(results, indent=2, sort_keys=True), encoding="utf-8"
    )
    lines = [
        "# AMOS Deterministic Benchmark Suite",
        "",
        f"Samples: {samples}. Retrieval distractors: {scale_items}.",
        "",
        "| System | Passed | Total | Pass Rate |",
        "| --- | ---: | ---: | ---: |",
    ]
    for system, aggregate in sorted(results["aggregate"].items()):
        lines.append(f"| {system} | {aggregate['passed']} | {aggregate['total']} | {aggregate['pass_rate']} |")
    lines.append("")
    (evaluation_dir / "benchmark_suite_summary.md").write_text("\n".join(lines), encoding="utf-8")
    return results


def _environment_metadata(config: dict[str, Any]) -> dict[str, Any]:
    package_names = ["duckdb", "fastapi", "matplotlib", "pydantic", "sqlglot", "typer", "uvicorn", "pytest"]
    versions = {}
    for name in package_names:
        try:
            versions[name] = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            versions[name] = "not-installed"
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "python": sys.version,
        "executable": sys.executable,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "packages": versions,
        "provider_credentials_present": {
            "OPENAI_API_KEY": bool(os.environ.get("OPENAI_API_KEY")),
            "ANTHROPIC_API_KEY": bool(os.environ.get("ANTHROPIC_API_KEY")),
            "GOOGLE_API_KEY": bool(os.environ.get("GOOGLE_API_KEY")),
        },
        "config": config,
    }


def _artifact_manifest(artifact_dir: Path) -> dict[str, Any]:
    files = []
    for path in sorted(artifact_dir.rglob("*")):
        if not path.is_file() or path.name in {
            "artifact_manifest.json",
            "bundle_manifest.json",
            "bundle_manifest.sha256",
        }:
            continue
        data = path.read_bytes()
        files.append(
            {
                "path": str(path.relative_to(artifact_dir)),
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "artifact_root": str(artifact_dir),
        "artifact_count": len(files),
        "total_bytes": sum(item["bytes"] for item in files),
        "files": files,
    }


def _write_source_manifest(evaluation_dir: Path) -> dict[str, Any]:
    project_root = Path(__file__).resolve().parents[2]
    include_roots = ["amos", "tests", "scenarios"]
    explicit_files = ["pyproject.toml"]
    paths: list[Path] = []
    for root_name in include_roots:
        root = project_root / root_name
        if root.exists():
            paths.extend(path for path in root.rglob("*") if path.is_file() and "__pycache__" not in path.parts)
    paths.extend(project_root / name for name in explicit_files if (project_root / name).is_file())
    files = []
    for path in sorted(set(paths)):
        data = path.read_bytes()
        files.append(
            {
                "path": str(path.relative_to(project_root)),
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    canonical = "\n".join(f"{item['path']}\0{item['sha256']}" for item in files).encode("utf-8")
    manifest = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "project_root": str(project_root),
        "source_file_count": len(files),
        "source_bytes": sum(item["bytes"] for item in files),
        "source_tree_sha256": hashlib.sha256(canonical).hexdigest(),
        "scope": "Evaluation implementation, tests, scenarios, and dependency declaration; manuscript/docs excluded to permit post-result manuscript finalization.",
        "files": files,
    }
    (evaluation_dir / "source_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return manifest


def _write_reproduction_record(root: Path, config: dict[str, Any]) -> None:
    systems = ",".join(config["systems"])
    systems_scale_sizes = ",".join(str(size) for size in config["systems_scale_sizes"])
    scale_argument = f"  --systems-scale-sizes {systems_scale_sizes} \\\n" if systems_scale_sizes else ""
    pilot_argument = f"  --live-pilot-dir {config['live_pilot_dir']} \\\n" if config["live_pilot_dir"] else ""
    retrieval_arguments = "".join(
        f"  --retrieval-comparison-dir {path} \\\n"
        for path in config["retrieval_comparison_dirs"]
    )
    text = f"""# AMOS Paper Bundle Reproduction Record

This directory was generated by the offline-capable paper harness.

```bash
python3 -m amos.evaluation.paper_bundle --all \\
  --run-dir {root} \\
  --variants {config['variants']} --samples {config['samples']} \\
  --generated-tasks {config['generated_tasks']} --variant-seed {config['variant_seed']} \\
  --scale-items {config['scale_items']} --concurrency {config['concurrency']} \\
{scale_argument}  --systems-scale-readers {config['systems_scale_readers']} --systems-scale-writes {config['systems_scale_writes']} \\
{pilot_argument}{retrieval_arguments}  --provider-mode {config['provider_mode']}
```

Systems: `{systems}`.

If no provider credential is present, provider-backed stages use the deterministic offline provider and preserve that limitation in every result file. Inspect `artifacts/evaluation/bundle_manifest.json`, `environment.json`, `artifact_manifest.json`, and `PAPER_RESULTS.md` before citing results.
"""
    (root / "REPRODUCTION.md").write_text(text, encoding="utf-8")


def _stage_summary(result: Any) -> Any:
    if isinstance(result, dict):
        if "aggregate" in result and isinstance(result["aggregate"], dict):
            aggregate = result["aggregate"]
            if aggregate and all(isinstance(item, dict) for item in aggregate.values()):
                total_runs = sum(int(item.get("runs", 0)) for item in aggregate.values())
                return {
                    "system_count": len(aggregate),
                    "total_runs": total_runs,
                    "amos": aggregate.get("amos", {}),
                }
            return aggregate
        for key in ["status", "returncode", "report_path"]:
            if key in result:
                return {key: result[key]}
        return {"keys": sorted(result.keys())}
    return str(result)


def _snapshot_settings() -> SettingsSnapshot:
    return SettingsSnapshot(
        root=settings.root,
        memory_db=settings.memory_db,
        analytics_db=settings.analytics_db,
        artifact_dir=settings.artifact_dir,
        rotate_analytics_db_on_seed=settings.rotate_analytics_db_on_seed,
    )


def _restore_settings(snapshot: SettingsSnapshot) -> None:
    for key, value in asdict(snapshot).items():
        setattr(settings, key, value)


def _reset_bundle_settings(root: Path) -> None:
    settings.use_run_dir(root)
    settings.ensure_dirs()


def _scale_label(size: int) -> str:
    if size >= 1_000_000 and size % 1_000_000 == 0:
        return f"{size // 1_000_000}m"
    if size >= 1_000 and size % 1_000 == 0:
        return f"{size // 1_000}k"
    return str(size)


def main() -> None:
    parser = argparse.ArgumentParser(description="Regenerate the AMOS systems-paper artifact bundle.")
    parser.add_argument("--all", action="store_true", help="Run the complete bundle (default behavior).")
    parser.add_argument("--run-dir", required=True)
    parser.add_argument("--scenarios-dir", default="scenarios")
    parser.add_argument("--variants", type=int, default=12)
    parser.add_argument("--samples", type=int, default=3)
    parser.add_argument("--generated-tasks", type=int, default=120)
    parser.add_argument("--variant-seed", type=int, default=20260711)
    parser.add_argument("--scale-items", type=int, default=5000)
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--benchmark-samples", type=int, default=3)
    parser.add_argument("--llm-samples", type=int, default=1)
    parser.add_argument("--claim-items", type=int, default=1000)
    parser.add_argument(
        "--systems-scale-sizes",
        default="",
        help="Comma-separated memory/provenance sizes, for example 10000,100000,1000000.",
    )
    parser.add_argument("--systems-scale-readers", type=int, default=8)
    parser.add_argument("--systems-scale-writes", type=int, default=64)
    parser.add_argument(
        "--live-pilot-dir",
        default=None,
        help="Archive an already executed live-pilot directory, including raw traces, into the bundle.",
    )
    parser.add_argument(
        "--retrieval-comparison-dir",
        action="append",
        default=[],
        help="Archive a pre-executed governed retrieval-comparison directory; repeat for multiple scales.",
    )
    parser.add_argument("--provider-mode", choices=["offline", "auto"], default="auto")
    parser.add_argument("--skip-tests", action="store_true")
    args = parser.parse_args()
    manifest = run_paper_bundle(
        run_dir=args.run_dir,
        scenarios_dir=args.scenarios_dir,
        variants=args.variants,
        samples=args.samples,
        generated_tasks=args.generated_tasks,
        variant_seed=args.variant_seed,
        scale_items=args.scale_items,
        concurrency=args.concurrency,
        benchmark_samples=args.benchmark_samples,
        llm_samples=args.llm_samples,
        claim_items=args.claim_items,
        systems_scale_sizes=[int(value) for value in args.systems_scale_sizes.split(",") if value.strip()],
        systems_scale_readers=args.systems_scale_readers,
        systems_scale_writes=args.systems_scale_writes,
        live_pilot_dir=args.live_pilot_dir,
        retrieval_comparison_dirs=args.retrieval_comparison_dir,
        provider_mode=args.provider_mode,
        run_tests=not args.skip_tests,
    )
    print(json.dumps(manifest, indent=2, sort_keys=True))
    if manifest["status"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
