from __future__ import annotations

from types import SimpleNamespace

from amos.evaluation.extended_experiments import _run_live_agent_llm_trials, run_extended_experiments


def test_extended_experiments_report_local_evidence() -> None:
    results = run_extended_experiments(
        scale_items=25,
        concurrency=2,
        llm_samples=1,
        write_artifacts=False,
    )

    assert "implemented_baselines" in results
    assert "noisy_retrieval_variants" in results
    assert "generated_benchmark_variants" in results
    assert "free_form_claim_extraction" in results
    assert "adversarial_security_suite" in results
    assert "scale_and_concurrency" in results
    assert results["implemented_baselines"]["baselines"]["implemented_catalog_lineage_dbt"]["aggregate"]["passed"] >= 4
    assert results["noisy_retrieval_variants"]["total"] == 5
    assert results["generated_benchmark_variants"]["total"] == 50
    assert results["generated_benchmark_variants"]["pass_rate"] >= 0.95
    assert results["free_form_claim_extraction"]["mean_type_recall"] >= 0.75
    assert results["free_form_claim_extraction"]["mean_type_precision"] >= 0.75
    assert results["free_form_claim_extraction"]["mean_review_obligation_recall"] >= 0.75
    assert results["adversarial_security_suite"]["total"] == 6
    assert results["scale_and_concurrency"]["scale_probe"]["target_retrieved"] is True
    assert results["scale_and_concurrency"]["concurrency"]["errors"] == []
    assert "production-scale robustness" in results["paper_readiness"]["still_cannot_claim"]


def test_live_agent_trials_use_supplied_provider(monkeypatch) -> None:
    supplied_provider = object()
    observed_providers = []
    monkeypatch.setattr("amos.evaluation.extended_experiments.seed_memory", lambda **kwargs: None)
    monkeypatch.setattr("amos.evaluation.extended_experiments.seed_duckdb", lambda: None)
    monkeypatch.setattr(
        "amos.evaluation.extended_experiments.provider_from_env",
        lambda: (_ for _ in ()).throw(AssertionError("provider_from_env must not replace an explicit provider")),
    )

    def fake_run(*args, provider=None, **kwargs):
        observed_providers.append(provider)
        return SimpleNamespace(
            status="pass",
            verification_status="pass",
            provider="test",
            model="test-model",
            raw_trace_path="trace.json",
        )

    monkeypatch.setattr("amos.evaluation.extended_experiments.run_live_agent_task", fake_run)
    trials = _run_live_agent_llm_trials(1, provider=supplied_provider)

    assert observed_providers == [supplied_provider]
    assert trials[0]["graded_pass"] is True
