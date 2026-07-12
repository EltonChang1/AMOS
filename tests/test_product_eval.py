from __future__ import annotations

import csv
import json
from pathlib import Path

import amos.evaluation.product_eval as product_eval_module
from amos.agent.live_agent import OfflineLiveProvider
from amos.evaluation.product_eval import run_product_eval


def test_product_eval_writes_reproducible_evidence_bundle(tmp_path: Path) -> None:
    results = run_product_eval(
        scenario="payment_failure",
        variants=1,
        samples=1,
        systems=["amos", "agent_only", "rag", "semantic", "catalog", "long_context"],
        run_dir=tmp_path / "product_eval",
        provider_mode="offline",
        write_artifacts=True,
    )

    output_dir = Path(results["output_dir"])
    assert (output_dir / "results.json").exists()
    assert (output_dir / "summary.md").exists()
    assert (output_dir / "failures.md").exists()
    assert (output_dir / "latency.csv").exists()
    assert (output_dir / "token_usage.csv").exists()
    assert (output_dir / "provenance_coverage.csv").exists()
    assert (output_dir / "family_metrics.csv").exists()
    assert (output_dir / "variant_manifest.json").exists()
    assert (output_dir / "variant_manifest.csv").exists()
    assert (output_dir / "paper_evidence.md").exists()

    records = results["records"]
    systems = {record["system"] for record in records}
    assert {"agent_only", "rag", "semantic", "catalog", "long_context"}.issubset(systems)
    assert results["aggregate"]["amos"]["pass_rate"] == 1.0
    assert results["aggregate"]["amos"]["statistical_unit"] == "seeded_variant"
    assert results["aggregate"]["amos"]["variants"] == 1
    assert results["aggregate"]["amos"]["variants_passed_all_samples"] == 1
    assert results["aggregate"]["amos"]["metric_means"]["provenance_coverage"] >= 0.95
    assert "metric_ci95" not in results["aggregate"]["amos"]
    assert "latency_seconds_mean_ci95" not in results["aggregate"]["amos"]
    assert results["aggregate"]["amos"]["metric_means"]["replay_success"] == 1.0
    assert "amos" in results["family_summary"]
    assert "end_to_end" in results["family_summary"]["amos"]
    assert results["tasks"][0]["variant_id"].startswith("payment_failure_variant_")
    assert results["tasks"][0]["perturbations"]
    assert results["records"][0]["variant_id"] == results["tasks"][0]["variant_id"]
    assert "failure_mode_counts" in results
    assert results["paper_evidence"]["offline_only_notice"]
    assert results["paper_evidence"]["raw_trace_paths"]

    for record in records:
        assert Path(record["raw_path"]).exists()
    for trace_path in results["paper_evidence"]["raw_trace_paths"]:
        assert Path(trace_path).exists()

    with (output_dir / "token_usage.csv").open(encoding="utf-8", newline="") as handle:
        token_rows = list(csv.DictReader(handle))
    assert token_rows
    assert all(int(row["total_tokens"]) > 0 for row in token_rows)
    with (output_dir / "variant_manifest.csv").open(encoding="utf-8", newline="") as handle:
        variant_rows = list(csv.DictReader(handle))
    assert len(variant_rows) == results["variant_count"]
    assert variant_rows[0]["perturbations"]

    summary = (output_dir / "summary.md").read_text(encoding="utf-8")
    assert "same-task comparison including policy-aware agent" in summary
    assert "no population confidence interval" in summary
    assert "do not claim live provider robustness" in summary
    paper_evidence = (output_dir / "paper_evidence.md").read_text(encoding="utf-8")
    assert "Claims Supported By Current Local Evidence" in paper_evidence
    assert "variant manifest" in paper_evidence
    assert "no population confidence interval" in paper_evidence


def test_subscription_product_eval_writes_scenario_specific_bundle(tmp_path: Path) -> None:
    results = run_product_eval(
        scenario="subscription_churn",
        variants=1,
        samples=1,
        systems=["amos", "agent_only", "rag", "semantic", "catalog", "long_context"],
        run_dir=tmp_path / "subscription_eval",
        provider_mode="offline",
        write_artifacts=True,
    )

    output_dir = Path(results["output_dir"])
    assert output_dir.name == "product_eval_subscription_churn"
    assert (output_dir / "results.json").exists()
    assert (output_dir / "paper_evidence.md").exists()
    assert results["scenario"] == "subscription_churn"
    assert results["adapter"] == "subscription_live_agent"
    assert results["aggregate"]["amos"]["pass_rate"] == 1.0
    assert results["aggregate"]["amos"]["metric_means"]["permission_safety"] == 1.0
    assert results["aggregate"]["amos"]["metric_means"]["replay_success"] == 1.0
    assert results["paper_evidence"]["raw_trace_paths"]
    trace = Path(results["paper_evidence"]["raw_trace_paths"][0])
    phases = [event["phase"] for event in json.loads(trace.read_text(encoding="utf-8"))["events"]]
    assert phases == [
        "retrieval_context",
        "analysis_plan",
        "sql_proposal",
        "tool_execution_and_verifier",
        "report_draft",
        "claims_and_replay",
    ]

    systems = {record["system"] for record in results["records"]}
    assert {"agent_only", "rag", "semantic", "catalog", "long_context"}.issubset(systems)
    for record in results["records"]:
        assert Path(record["raw_path"]).exists()


def test_warehouse_product_eval_writes_scenario_specific_bundle(tmp_path: Path) -> None:
    results = run_product_eval(
        scenario="warehouse_quality",
        variants=1,
        samples=1,
        systems=["amos", "agent_only", "rag", "semantic", "catalog", "long_context"],
        run_dir=tmp_path / "warehouse_eval",
        provider_mode="offline",
        write_artifacts=True,
    )

    output_dir = Path(results["output_dir"])
    assert output_dir.name == "product_eval_warehouse_quality"
    assert (output_dir / "results.json").exists()
    assert (output_dir / "paper_evidence.md").exists()
    assert results["scenario"] == "warehouse_quality"
    assert results["adapter"] == "warehouse_live_agent"
    assert results["aggregate"]["amos"]["pass_rate"] == 1.0
    assert results["aggregate"]["amos"]["metric_means"]["permission_safety"] == 1.0
    assert results["aggregate"]["amos"]["metric_means"]["replay_success"] == 1.0
    assert results["paper_evidence"]["raw_trace_paths"]
    trace = Path(results["paper_evidence"]["raw_trace_paths"][0])
    phases = [event["phase"] for event in json.loads(trace.read_text(encoding="utf-8"))["events"]]
    assert phases == [
        "retrieval_context",
        "analysis_plan",
        "sql_proposal",
        "tool_execution_and_verifier",
        "report_draft",
        "claims_and_replay",
    ]

    systems = {record["system"] for record in results["records"]}
    assert {"agent_only", "rag", "semantic", "catalog", "long_context"}.issubset(systems)
    for record in results["records"]:
        assert Path(record["raw_path"]).exists()


def test_cross_domain_auto_mode_calls_configured_provider(monkeypatch, tmp_path: Path) -> None:
    class RecordingProvider(OfflineLiveProvider):
        def __init__(self) -> None:
            self.phases: list[str] = []

        def complete(self, prompt: str, *, phase: str, response_format: str = "text"):
            self.phases.append(phase)
            return super().complete(prompt, phase=phase, response_format=response_format)

    provider = RecordingProvider()
    monkeypatch.setattr(product_eval_module, "provider_from_env", lambda: provider)

    results = run_product_eval(
        scenario="subscription_churn",
        variants=1,
        samples=1,
        systems=["amos"],
        run_dir=tmp_path / "provider_routing",
        provider_mode="auto",
        write_artifacts=True,
    )

    assert results["aggregate"]["amos"]["pass_rate"] == 1.0
    assert provider.phases == ["analysis_plan", "sql_proposal", "report_draft"]


def test_strong_baselines_and_ablations_target_distinct_guarantees(tmp_path: Path) -> None:
    systems = [
        "amos",
        "agent_with_manual_policy_prompt",
        "rag_with_permission_filter",
        "amos_no_verifier",
        "amos_no_permission_gate",
        "amos_no_provenance",
    ]
    results = run_product_eval(
        scenario="payment_failure",
        variants=4,
        samples=1,
        systems=systems,
        run_dir=tmp_path / "comparison",
        provider_mode="offline",
        write_artifacts=True,
    )

    aggregate = results["aggregate"]
    for system in ["agent_with_manual_policy_prompt", "rag_with_permission_filter"]:
        metrics = aggregate[system]["metric_means"]
        assert metrics["task_correctness"] == 1.0
        assert metrics["metric_correctness"] == 1.0
        assert metrics["schema_correctness"] == 1.0
        assert metrics["permission_safety"] == 1.0
        assert metrics["provenance_coverage"] == 0.0
        assert metrics["replay_success"] == 0.0

    assert aggregate["amos_no_permission_gate"]["metric_means"]["permission_safety"] == 0.0
    assert aggregate["amos_no_permission_gate"]["metric_means"]["replay_success"] == 1.0
    assert aggregate["amos_no_verifier"]["metric_means"]["metric_correctness"] < 1.0
    assert aggregate["amos_no_verifier"]["metric_means"]["provenance_coverage"] == 1.0
    no_provenance = aggregate["amos_no_provenance"]["metric_means"]
    assert no_provenance["task_correctness"] == 1.0
    assert no_provenance["metric_correctness"] == 1.0
    assert no_provenance["permission_safety"] == 1.0
    assert no_provenance["provenance_coverage"] == 0.0
    assert no_provenance["replay_success"] == 1.0

    output_dir = Path(results["output_dir"])
    assert (output_dir / "system_contracts.json").exists()
    assert (output_dir / "system_contracts.csv").exists()
    assert (output_dir / "metric_axis_summary.csv").exists()
    assert (output_dir / "failure_modes.csv").exists()
    assert (output_dir / "provenance_overhead.json").exists()
    assert (output_dir / "provenance_overhead.csv").exists()
    assert results["provenance_overhead"]["pair_count"] == 4
    assert results["provenance_overhead"]["statistical_unit"] == "seeded_variant_mean_across_repeats"
    assert "seeded_variant_bootstrap_interval95" in results["provenance_overhead"]["summary"]["latency_delta_seconds"]
    assert len(results["paper_evidence"]["raw_trace_paths"]) == 8
    manual_record = next(record for record in results["records"] if record["system"] == "agent_with_manual_policy_prompt")
    raw = json.loads(Path(manual_record["raw_path"]).read_text(encoding="utf-8"))
    assert "Current approved policy/schema/metric context" in raw["verbatim_prompt"]
    assert raw["system_contract"]["runtime_verifier"] is False
    rag_record = next(record for record in results["records"] if record["system"] == "rag_with_permission_filter")
    rag_raw = json.loads(Path(rag_record["raw_path"]).read_text(encoding="utf-8"))
    assert "permission-filtered RAG baseline" in rag_raw["verbatim_prompt"]
    assert rag_raw["restricted_memory_in_context"] == []
