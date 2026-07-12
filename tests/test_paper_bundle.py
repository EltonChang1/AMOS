from __future__ import annotations

from pathlib import Path

import amos.evaluation.paper_bundle as paper_bundle


def test_paper_bundle_writes_reproducibility_manifests(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(paper_bundle, "load_all_scenario_packs", lambda **kwargs: {"status": "ok"})
    monkeypatch.setattr(paper_bundle, "_run_scenario_pack_stage", lambda *args: {"status": "ok"})
    monkeypatch.setattr(paper_bundle, "run_product_eval", lambda **kwargs: {"status": "ok"})
    monkeypatch.setattr(paper_bundle, "_run_benchmark_stage", lambda *args: {"status": "ok"})
    monkeypatch.setattr(paper_bundle, "run_extended_experiments", lambda **kwargs: {"status": "ok"})

    def fake_report(evaluation_dir: Path):
        path = Path(evaluation_dir) / "PAPER_RESULTS.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("# Paper results\n", encoding="utf-8")
        return {"report_path": str(path)}

    monkeypatch.setattr(paper_bundle, "generate_paper_results_report", fake_report)
    run_dir = tmp_path / "bundle"
    manifest = paper_bundle.run_paper_bundle(
        run_dir=run_dir,
        variants=1,
        samples=1,
        generated_tasks=1,
        scale_items=0,
        concurrency=1,
        benchmark_samples=1,
        llm_samples=1,
        provider_mode="offline",
        run_tests=False,
    )

    evaluation = run_dir / "artifacts" / "evaluation"
    assert manifest["status"] == "pass"
    assert (evaluation / "bundle_manifest.json").exists()
    assert (evaluation / "bundle_manifest.sha256").exists()
    assert (evaluation / "environment.json").exists()
    assert (evaluation / "source_manifest.json").exists()
    assert (evaluation / "artifact_manifest.json").exists()
    assert (evaluation / "PAPER_RESULTS.md").exists()
    assert (run_dir / "REPRODUCTION.md").exists()
    assert [stage["name"] for stage in manifest["stages"]] == [
        "source_manifest",
        "unit_tests",
        "scenario_loads",
        "evidence_schemas",
        "scenario_packs",
        "product_eval_payment_failure",
        "product_eval_subscription_churn",
        "product_eval_warehouse_quality",
        "benchmark_suite",
        "extended_experiments",
        "paper_report",
    ]
