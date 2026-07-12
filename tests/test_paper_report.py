from __future__ import annotations

import json
from pathlib import Path

from amos.evaluation.paper_report import generate_paper_results_report


def test_paper_report_consolidates_required_artifacts(tmp_path: Path) -> None:
    evaluation_dir = tmp_path / "evaluation"
    product_dir = evaluation_dir / "product_eval"
    raw_dir = product_dir / "raw"
    raw_dir.mkdir(parents=True)
    trace_path = tmp_path / "llm_trace.json"
    raw_path = raw_dir / "amos_raw.json"
    trace_path.write_text("{}", encoding="utf-8")
    raw_path.write_text("{}", encoding="utf-8")
    _write_text_artifacts(product_dir)
    _write_json(product_dir / "results.json", _product_results(raw_path, trace_path))
    _write_json(evaluation_dir / "benchmark_suite.json", _benchmark_results())
    _write_json(evaluation_dir / "extended_experiments.json", _extended_results())
    (evaluation_dir / "benchmark_suite_summary.md").write_text("# Benchmark\n", encoding="utf-8")
    (evaluation_dir / "extended_experiments_summary.md").write_text("# Extended\n", encoding="utf-8")

    index = generate_paper_results_report(evaluation_dir)

    report_path = Path(index["report_path"])
    index_path = evaluation_dir / "paper_artifact_index.json"
    assert report_path.exists()
    assert index_path.exists()
    assert index["inventory"]["raw_evidence_existing"] == 1
    assert index["inventory"]["raw_trace_existing"] == 1

    report = report_path.read_text(encoding="utf-8")
    assert "AMOS Paper Results Draft" in report
    assert "Capability-Contract Evaluation" in report
    assert "Deterministic Benchmark Suite" in report
    assert "Extended Experiments" in report
    assert "Claims Supported For Paper Draft" in report
    assert "Claims Not Yet Supported" in report


def test_paper_report_includes_scenario_pack_artifacts_when_present(tmp_path: Path) -> None:
    evaluation_dir = tmp_path / "evaluation"
    product_dir = evaluation_dir / "product_eval"
    raw_dir = product_dir / "raw"
    raw_dir.mkdir(parents=True)
    trace_path = tmp_path / "llm_trace.json"
    raw_path = raw_dir / "amos_raw.json"
    trace_path.write_text("{}", encoding="utf-8")
    raw_path.write_text("{}", encoding="utf-8")
    _write_text_artifacts(product_dir)
    _write_json(product_dir / "results.json", _product_results(raw_path, trace_path))
    extra_product_dir = evaluation_dir / "product_eval_subscription_churn"
    _write_text_artifacts(extra_product_dir)
    extra_product = _product_results(raw_path, trace_path)
    extra_product["scenario"] = "subscription_churn"
    extra_product["adapter"] = "subscription_deterministic_product_adapter"
    _write_json(extra_product_dir / "results.json", extra_product)
    _write_json(evaluation_dir / "benchmark_suite.json", _benchmark_results())
    _write_json(evaluation_dir / "extended_experiments.json", _extended_results())
    (evaluation_dir / "benchmark_suite_summary.md").write_text("# Benchmark\n", encoding="utf-8")
    (evaluation_dir / "extended_experiments_summary.md").write_text("# Extended\n", encoding="utf-8")
    scenario_dir = evaluation_dir / "scenario_packs"
    _write_json(scenario_dir / "scenario_pack_report.json", _scenario_pack_results())
    (scenario_dir / "scenario_pack_summary.md").write_text("# Scenario Packs\n", encoding="utf-8")
    (scenario_dir / "scenario_pack_coverage.csv").write_text("pack_id\npayment_failure\n", encoding="utf-8")
    _write_json(scenario_dir / "generated_tasks.json", _generated_task_results())
    (scenario_dir / "generated_tasks_summary.md").write_text("# Generated Tasks\n", encoding="utf-8")
    (scenario_dir / "generated_tasks.csv").write_text("variant_id\nv1\n", encoding="utf-8")
    load_dir = evaluation_dir / "scenario_loads"
    _write_json(load_dir / "scenario_load_report.json", _scenario_load_results())
    (load_dir / "scenario_load_summary.md").write_text("# Scenario Loads\n", encoding="utf-8")

    index = generate_paper_results_report(evaluation_dir)

    assert "scenario_packs" in index["generated_from"]
    assert "generated_scenario_tasks" in index["generated_from"]
    assert "scenario_loads" in index["generated_from"]
    assert "product_eval_subscription_churn" in index["generated_from"]
    report = Path(index["report_path"]).read_text(encoding="utf-8")
    assert "Scenario Packs" in report
    assert "Generated Scenario Tasks" in report
    assert "Scenario Loader" in report
    assert "Additional Capability-Contract Evaluations" in report
    assert "Versioned scenario manifests cover 1 domains" in report


def test_paper_report_describes_completed_live_pilot_without_robustness_claim(tmp_path: Path) -> None:
    evaluation_dir = tmp_path / "evaluation"
    product_dir = evaluation_dir / "product_eval"
    raw_dir = product_dir / "raw"
    raw_dir.mkdir(parents=True)
    trace_path = tmp_path / "llm_trace.json"
    raw_path = raw_dir / "amos_raw.json"
    trace_path.write_text("{}", encoding="utf-8")
    raw_path.write_text("{}", encoding="utf-8")
    _write_text_artifacts(product_dir)
    _write_json(product_dir / "results.json", _product_results(raw_path, trace_path))
    _write_json(evaluation_dir / "benchmark_suite.json", _benchmark_results())
    _write_json(evaluation_dir / "extended_experiments.json", _extended_results())
    (evaluation_dir / "benchmark_suite_summary.md").write_text("# Benchmark\n", encoding="utf-8")
    (evaluation_dir / "extended_experiments_summary.md").write_text("# Extended\n", encoding="utf-8")
    _write_json(
        evaluation_dir / "live_llm_pilot" / "results.json",
        {
            "status": "completed",
            "provider": "provider-a",
            "model": "model-a",
            "provider_failures": 0,
            "provider_attempt_failures": 1,
            "policy_trials": {
                "intended": 8,
                "completed": 8,
                "graded_passed": 3,
                "trials": [{} for _ in range(8)],
            },
            "live_agent_trials": {
                "intended": 3,
                "completed": 3,
                "graded_passed": 3,
                "trials": [{} for _ in range(3)],
            },
        },
    )
    retrieval_metrics = {
        "top1_accuracy": 0.5,
        "recall_at_5": 0.75,
        "mean_reciprocal_rank": 0.6,
        "p50_latency_seconds": 0.01,
        "p95_latency_seconds": 0.02,
        "permission_leak_count": 0,
        "superseded_leak_count": 0,
    }
    _write_json(
        evaluation_dir / "retrieval_engine_comparison" / "archive_manifest.json",
        {
            "runs": [
                {
                    "distractors": 1_000,
                    "aggregate": {
                        "bm25_governed": retrieval_metrics,
                        "minilm_hnsw_governed": retrieval_metrics,
                        "rrf_hybrid_governed": retrieval_metrics,
                    },
                }
            ]
        },
    )

    index = generate_paper_results_report(evaluation_dir)
    report = Path(index["report_path"]).read_text(encoding="utf-8")

    assert "End-to-end tasks | 3/3 | 3" in report
    assert "Provider-attempt failures preserved | 1" in report
    assert "supports a narrow end-to-end feasibility claim" in report
    assert "end-to-end graded 3/3; policy rubric 3/8" in report
    assert "does not support live-model robustness" in report
    assert "Governed Retrieval-Engine Comparison" in report
    assert "| 1000 | 0.5 | 0.02 |" in report
    assert "Production vector/hybrid superiority" in report


def _write_json(path: Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")


def _write_text_artifacts(product_dir: Path) -> None:
    product_dir.mkdir(parents=True, exist_ok=True)
    for name in [
        "summary.md",
        "paper_evidence.md",
        "failures.md",
        "latency.csv",
        "token_usage.csv",
        "provenance_coverage.csv",
        "family_metrics.csv",
        "variant_manifest.csv",
    ]:
        (product_dir / name).write_text("placeholder\n", encoding="utf-8")
    _write_json(product_dir / "variant_manifest.json", {"variants": []})


def _product_results(raw_path: Path, trace_path: Path) -> dict[str, object]:
    return {
        "scenario": "payment_failure",
        "variant_count": 1,
        "variant_seed": 20260711,
        "samples": 1,
        "systems": ["amos"],
        "provider": "offline",
        "model": "offline-structured-live-agent",
        "tasks": [{"perturbations": ["direct_wording"]}],
        "aggregate": {
            "amos": {
                "runs": 1,
                "passed": 1,
                "pass_rate": 1.0,
                "variants": 1,
                "variants_passed_all_samples": 1,
                "latency_seconds_mean": 0.1,
                "metric_means": {"provenance_coverage": 1.0, "replay_success": 1.0},
                "token_usage": {"total_tokens": 10},
            }
        },
        "paper_evidence": {
            "offline_only_notice": "offline provider",
            "raw_evidence_paths": [str(raw_path)],
            "raw_trace_paths": [str(trace_path)],
        },
    }


def _benchmark_results() -> dict[str, object]:
    return {
        "aggregate": {"amos": {"passed": 1, "total": 1, "pass_rate": 1.0}},
        "scale_probe": {
            "memory_objects_added": 100,
            "target_retrieved": True,
            "target_rank": 1,
            "retrieval_seconds": 0.1,
        },
    }


def _extended_results() -> dict[str, object]:
    return {
        "noisy_retrieval_variants": {"passed": 1, "total": 1},
        "generated_benchmark_variants": {"passed": 1, "total": 1},
        "free_form_claim_extraction": {"mean_type_precision": 1.0, "mean_type_recall": 1.0},
        "adversarial_security_suite": {"passed": 1, "total": 1},
        "scale_and_concurrency": {
            "scale_probe": {"retrieval_seconds": 0.1},
            "concurrency": {"p95_latency_seconds": 0.2},
        },
        "live_llm_trials": {"status": "skipped"},
    }


def _scenario_pack_results() -> dict[str, object]:
    return {
        "aggregate": {
            "pack_count": 1,
            "executable_product_eval_count": 1,
            "live_agent_ready_count": 1,
        },
        "packs": [
            {
                "pack_id": "payment_failure",
                "domain": "payments analytics",
                "status": "executable_product_eval",
                "task_count": 4,
                "manifest_completeness_score": 1.0,
                "execution_readiness_score": 1.0,
            }
        ],
        "paper_claim_boundary": [
            "Versioned scenario manifests cover 1 domains: payments analytics.",
        ],
    }


def _generated_task_results() -> dict[str, object]:
    return {
        "seed": 20260711,
        "variant_count": 100,
        "aggregate": {
            "runs": 100,
            "manifest_contract_passed": 100,
            "product_eval_executable_runs": 100,
            "runtime_seeded_pending_adapter_runs": 0,
            "manifest_only_runs": 0,
            "contract_failed_runs": 0,
            "raw_evidence_count": 100,
        },
        "paper_claim_boundary": [
            "Generated 100 cross-domain task variants from seed-controlled scenario manifests.",
        ],
    }


def _scenario_load_results() -> dict[str, object]:
    return {
        "aggregate": {
            "pack_count": 1,
            "runtime_seeded_count": 1,
            "manifest_only_count": 0,
        },
        "reports": [
            {
                "scenario_id": "payment_failure",
                "status": "runtime_seeded",
                "task_count": 4,
                "materialized_runtime": {"runtime_seeded": True},
            }
        ],
        "paper_claim_boundary": [
            "Scenario loader materialized 1 scenario bundles.",
        ],
    }
