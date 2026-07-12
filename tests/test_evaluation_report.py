from __future__ import annotations

from amos.evaluation.run_eval import run_all


def test_evaluation_reports_scenarios_and_ablations() -> None:
    results = run_all(samples=1, scale_items=100)

    task_names = {task["name"] for task in results["tasks"]}  # type: ignore[index]
    assert {"metric_drift", "schema_drift", "prompt_injection", "retrieval_scale"}.issubset(task_names)
    assert results["aggregate"]["amos"]["pass_rate"] == 1.0  # type: ignore[index]
    assert results["amos_task_status"]["payment_failure_spike"] == "warning"  # type: ignore[index]
    assert results["amos_task_status"]["causal_review"] == "needs_review"  # type: ignore[index]
    assert results["amos_task_status"]["schema_drift"] == "rejected_stale_sql"  # type: ignore[index]
    assert results["task_protocol"][0]["oracle"]  # type: ignore[index]
    assert results["task_protocol"][0]["expected_status"]  # type: ignore[index]
    assert results["baseline_profiles"]["catalog_lineage_dbt_agent"]["type"] == "modeled_policy"  # type: ignore[index]
    assert "semantic_layer_agent" in results["aggregate"]  # type: ignore[operator]
    assert results["baseline_task_pass"]["catalog_lineage_dbt_agent"]["schema_drift"] is True  # type: ignore[index]
    assert results["baseline_task_pass"]["metadata_rag_access_control"]["permission_conflict"] is True  # type: ignore[index]
    assert results["baseline_task_pass"]["tool_llm_structured"]["prompt_injection"] is False  # type: ignore[index]
    assert results["ablation_summary"]["remove_verifier"]["delta_passes"] < 0  # type: ignore[index]
    assert results["scale_probe"]["target_retrieved"] is True  # type: ignore[index]
    assert len(results["scale_probe"]["sensitivity"]) >= 2  # type: ignore[index]
    assert results["scale_probe"]["sensitivity"][-1]["target_rank"] == 1  # type: ignore[index]
    assert results["security_checks"]["memory_poisoning_blocked"] is True  # type: ignore[index]
    assert results["llm_agent_experiment"]["mode"] == "offline_structured_simulation"  # type: ignore[index]
