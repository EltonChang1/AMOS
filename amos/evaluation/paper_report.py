from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from amos.config import settings


def generate_paper_results_report(
    evaluation_dir: str | Path | None = None,
    output_path: str | Path | None = None,
) -> dict[str, Any]:
    root = Path(evaluation_dir or settings.artifact_dir / "evaluation").resolve()
    product_path = root / "product_eval" / "results.json"
    extra_product_paths = sorted(root.glob("product_eval_*/results.json"))
    benchmark_path = root / "benchmark_suite.json"
    extended_path = root / "extended_experiments.json"
    scenario_packs_path = root / "scenario_packs" / "scenario_pack_report.json"
    generated_tasks_path = root / "scenario_packs" / "generated_tasks.json"
    scenario_load_path = root / "scenario_loads" / "scenario_load_report.json"
    live_pilot_path = root / "live_llm_pilot" / "results.json"
    retrieval_comparison_path = root / "retrieval_engine_comparison" / "archive_manifest.json"
    required = [product_path, benchmark_path, extended_path]
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise FileNotFoundError(f"Missing required evaluation artifacts: {missing}")

    product = _read_json(product_path)
    extra_products = [_read_json(path) for path in extra_product_paths]
    benchmark = _read_json(benchmark_path)
    extended = _read_json(extended_path)
    scenario_packs = _read_json(scenario_packs_path) if scenario_packs_path.exists() else None
    generated_tasks = _read_json(generated_tasks_path) if generated_tasks_path.exists() else None
    scenario_loads = _read_json(scenario_load_path) if scenario_load_path.exists() else None
    live_pilot = _read_json(live_pilot_path) if live_pilot_path.exists() else None
    retrieval_comparisons = (
        _read_json(retrieval_comparison_path) if retrieval_comparison_path.exists() else None
    )
    systems_scale_paths = sorted(root.glob("systems_scale*/results.json"))
    systems_scales = [_read_json(path) for path in systems_scale_paths]
    output = Path(output_path or root / "PAPER_RESULTS.md").resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    inventory = _artifact_inventory(
        root,
        product,
        generated_tasks,
        extra_products,
        systems_scale_paths,
        live_pilot_path,
        retrieval_comparison_path,
    )
    report = _render_report(
        root,
        product,
        extra_products,
        benchmark,
        extended,
        inventory,
        scenario_packs,
        generated_tasks,
        scenario_loads,
        systems_scales,
        live_pilot,
        retrieval_comparisons,
    )
    output.write_text(report, encoding="utf-8")
    generated_from = {
        "product_eval": str(product_path),
        "benchmark_suite": str(benchmark_path),
        "extended_experiments": str(extended_path),
    }
    for path in extra_product_paths:
        generated_from[f"product_eval_{path.parent.name.removeprefix('product_eval_')}"] = str(path)
    if scenario_packs_path.exists():
        generated_from["scenario_packs"] = str(scenario_packs_path)
    if generated_tasks_path.exists():
        generated_from["generated_scenario_tasks"] = str(generated_tasks_path)
    if scenario_load_path.exists():
        generated_from["scenario_loads"] = str(scenario_load_path)
    for path in systems_scale_paths:
        generated_from[path.parent.name] = str(path)
    if live_pilot_path.exists():
        generated_from["live_llm_pilot"] = str(live_pilot_path)
    if retrieval_comparison_path.exists():
        generated_from["retrieval_engine_comparison"] = str(retrieval_comparison_path)
    index = {
        "report_path": str(output),
        "evaluation_dir": str(root),
        "inventory": inventory,
        "generated_from": generated_from,
    }
    (root / "paper_artifact_index.json").write_text(json.dumps(index, indent=2, sort_keys=True), encoding="utf-8")
    return index


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _artifact_inventory(
    root: Path,
    product: dict[str, Any],
    generated_tasks: dict[str, Any] | None,
    extra_products: list[dict[str, Any]],
    systems_scale_paths: list[Path],
    live_pilot_path: Path,
    retrieval_comparison_path: Path,
) -> dict[str, Any]:
    named = {
        "product_results": root / "product_eval" / "results.json",
        "product_summary": root / "product_eval" / "summary.md",
        "product_paper_evidence": root / "product_eval" / "paper_evidence.md",
        "product_failures": root / "product_eval" / "failures.md",
        "latency_csv": root / "product_eval" / "latency.csv",
        "token_usage_csv": root / "product_eval" / "token_usage.csv",
        "provenance_csv": root / "product_eval" / "provenance_coverage.csv",
        "family_metrics_csv": root / "product_eval" / "family_metrics.csv",
        "variant_manifest_json": root / "product_eval" / "variant_manifest.json",
        "variant_manifest_csv": root / "product_eval" / "variant_manifest.csv",
        "system_contracts_json": root / "product_eval" / "system_contracts.json",
        "system_contracts_csv": root / "product_eval" / "system_contracts.csv",
        "metric_axis_summary_csv": root / "product_eval" / "metric_axis_summary.csv",
        "failure_modes_csv": root / "product_eval" / "failure_modes.csv",
        "provenance_overhead_json": root / "product_eval" / "provenance_overhead.json",
        "provenance_overhead_csv": root / "product_eval" / "provenance_overhead.csv",
        "benchmark_results": root / "benchmark_suite.json",
        "benchmark_summary": root / "benchmark_suite_summary.md",
        "extended_results": root / "extended_experiments.json",
        "extended_summary": root / "extended_experiments_summary.md",
        "scenario_pack_report": root / "scenario_packs" / "scenario_pack_report.json",
        "scenario_pack_summary": root / "scenario_packs" / "scenario_pack_summary.md",
        "scenario_pack_coverage": root / "scenario_packs" / "scenario_pack_coverage.csv",
        "generated_scenario_tasks": root / "scenario_packs" / "generated_tasks.json",
        "generated_scenario_tasks_summary": root / "scenario_packs" / "generated_tasks_summary.md",
        "generated_scenario_tasks_csv": root / "scenario_packs" / "generated_tasks.csv",
        "scenario_load_report": root / "scenario_loads" / "scenario_load_report.json",
        "scenario_load_summary": root / "scenario_loads" / "scenario_load_summary.md",
        "evidence_schema_manifest": root / "evidence_schemas" / "schema_manifest.json",
        "independent_task_schema": root / "evidence_schemas" / "independent_task_study.schema.json",
        "task_prediction_schema": root / "evidence_schemas" / "task_predictions.schema.json",
        "claim_annotation_schema": root / "evidence_schemas" / "claim_annotation_study.schema.json",
        "claim_prediction_schema": root / "evidence_schemas" / "claim_predictions.schema.json",
        "external_product_schema": root / "evidence_schemas" / "external_product_study.schema.json",
        "live_pilot_results": live_pilot_path,
        "live_pilot_summary": live_pilot_path.parent / "summary.md",
        "live_pilot_archive_manifest": live_pilot_path.parent / "archive_manifest.json",
        "retrieval_engine_comparison_archive_manifest": retrieval_comparison_path,
    }
    for extra_product in extra_products:
        scenario = str(extra_product.get("scenario", "unknown"))
        directory = Path(str(extra_product.get("output_dir", root / f"product_eval_{scenario}")))
        prefix = f"product_eval_{scenario}"
        named[f"{prefix}_results"] = directory / "results.json"
        named[f"{prefix}_summary"] = directory / "summary.md"
        named[f"{prefix}_paper_evidence"] = directory / "paper_evidence.md"
        named[f"{prefix}_failures"] = directory / "failures.md"
        named[f"{prefix}_system_contracts"] = directory / "system_contracts.json"
        named[f"{prefix}_metric_axis_summary"] = directory / "metric_axis_summary.csv"
        named[f"{prefix}_failure_modes"] = directory / "failure_modes.csv"
        named[f"{prefix}_provenance_overhead"] = directory / "provenance_overhead.json"
    for path in systems_scale_paths:
        prefix = path.parent.name
        named[f"{prefix}_results"] = path
        named[f"{prefix}_summary"] = path.parent / "summary.md"
        named[f"{prefix}_sha256"] = path.parent / "results.sha256"
    raw_paths = [Path(path) for path in product.get("paper_evidence", {}).get("raw_evidence_paths", [])]
    trace_paths = [Path(path) for path in product.get("paper_evidence", {}).get("raw_trace_paths", [])]
    for extra_product in extra_products:
        raw_paths.extend(Path(path) for path in extra_product.get("paper_evidence", {}).get("raw_evidence_paths", []))
        trace_paths.extend(Path(path) for path in extra_product.get("paper_evidence", {}).get("raw_trace_paths", []))
    generated_raw_paths = [
        Path(record["raw_path"])
        for record in (generated_tasks or {}).get("records", [])
        if record.get("raw_path")
    ]
    return {
        "named_artifacts": {
            name: {"path": str(path), "exists": path.exists()}
            for name, path in named.items()
        },
        "raw_evidence_count": len(raw_paths),
        "raw_evidence_existing": sum(1 for path in raw_paths if path.exists()),
        "raw_trace_count": len(trace_paths),
        "raw_trace_existing": sum(1 for path in trace_paths if path.exists()),
        "generated_raw_evidence_count": len(generated_raw_paths),
        "generated_raw_evidence_existing": sum(1 for path in generated_raw_paths if path.exists()),
    }


def _render_report(
    root: Path,
    product: dict[str, Any],
    extra_products: list[dict[str, Any]],
    benchmark: dict[str, Any],
    extended: dict[str, Any],
    inventory: dict[str, Any],
    scenario_packs: dict[str, Any] | None,
    generated_tasks: dict[str, Any] | None,
    scenario_loads: dict[str, Any] | None,
    systems_scales: list[dict[str, Any]],
    live_pilot: dict[str, Any] | None,
    retrieval_comparisons: dict[str, Any] | None,
) -> str:
    lines: list[str] = [
        "# AMOS Paper Results Draft",
        "",
        "This report is generated from raw evaluation artifacts. It is intentionally conservative: claims are listed only where the current artifact bundle supports them, and open evidence gaps remain visible.",
        "",
        "## Artifact Inventory",
        "",
        "| Artifact | Exists | Path |",
        "| --- | ---: | --- |",
    ]
    for name, item in sorted(inventory["named_artifacts"].items()):
        lines.append(f"| {name} | {str(item['exists']).lower()} | `{_rel(root, item['path'])}` |")
    lines.extend(
        [
            f"| raw_evidence_files | {inventory['raw_evidence_existing']}/{inventory['raw_evidence_count']} | `product_eval/raw/*.json` |",
            f"| raw_llm_trace_files | {inventory['raw_trace_existing']}/{inventory['raw_trace_count']} | see `product_eval/results.json` |",
            f"| generated_scenario_raw_files | {inventory['generated_raw_evidence_existing']}/{inventory['generated_raw_evidence_count']} | `scenario_packs/generated_raw/*.json` |",
            "",
            "## Capability-Contract Evaluation",
            "",
            f"Scenario: `{product['scenario']}`.",
            f"Variants: {product['variant_count']} with seed `{product.get('variant_seed')}`.",
            f"Samples: {product['samples']}. Systems: {', '.join(product['systems'])}.",
            f"Provider: `{product['provider']}` / `{product['model']}`.",
            "",
            "Full-contract pass requires analytical correctness, permission safety, the review boundary, claim provenance, and replay; it is a capability-contract score, not a product-superiority or SQL-only score.",
            "",
            "Rates are descriptive across seeded variants. Deterministic repeated samples are not independent, so no population confidence interval is reported.",
            "",
            "| System | Variants | Variants Passing All Samples | Variant Pass Rate | Executions | Mean Latency (s) | Provenance | Replay | Total Tokens |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for system, aggregate in sorted(product["aggregate"].items()):
        metrics = aggregate["metric_means"]
        fallback_variant_passed = int(aggregate["passed"]) // max(int(product.get("samples", 1)), 1)
        lines.append(
            f"| {system} | {aggregate.get('variants', product['variant_count'])} | "
            f"{aggregate.get('variants_passed_all_samples', fallback_variant_passed)} | {aggregate['pass_rate']} | "
            f"{aggregate['runs']} | {aggregate['latency_seconds_mean']} | "
            f"{metrics['provenance_coverage']} | {metrics['replay_success']} | {aggregate['token_usage']['total_tokens']} |"
        )
    _append_metric_axis_table(lines, product)
    _append_system_contract_table(lines, product)
    _append_provenance_overhead_table(lines, product)
    lines.extend(
        [
            "",
            "### Capability-Evaluation Claim Boundary",
            "",
            f"- {product['paper_evidence'].get('offline_only_notice') or 'Provider-backed live trials completed according to product_eval results.'}",
            "- Cross-domain live-agent evidence is reported separately below; all offline-provider results remain bounded from provider-backed claims.",
            "- Baselines are executable local approximations, not complete enterprise product integrations or evidence of product superiority.",
            "",
            "### Perturbation Manifest",
            "",
            "| Perturbation | Count |",
            "| --- | ---: |",
        ]
    )
    for perturbation, count in sorted(_perturbation_counts(product.get("tasks", [])).items()):
        lines.append(f"| {perturbation} | {count} |")

    if extra_products:
        lines.extend(["", "## Additional Capability-Contract Evaluations", ""])
        for extra_product in extra_products:
            lines.extend(
                [
                    f"Scenario: `{extra_product['scenario']}`.",
                    f"Variants: {extra_product['variant_count']} with seed `{extra_product.get('variant_seed')}`.",
                    f"Adapter: `{extra_product.get('adapter', 'unknown')}`. Provider: `{extra_product['provider']}` / `{extra_product['model']}`.",
                    "",
                    "Full-contract pass requires every reported guarantee; see the metric-axis table for partial capability.",
                    "",
                    "| System | Variants | Variants Passing All Samples | Variant Pass Rate | Executions | Provenance | Replay |",
                    "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
                ]
            )
            for system, aggregate in sorted(extra_product["aggregate"].items()):
                metrics = aggregate["metric_means"]
                fallback_variant_passed = int(aggregate["passed"]) // max(int(extra_product.get("samples", 1)), 1)
                lines.append(
                    f"| {system} | {aggregate.get('variants', extra_product['variant_count'])} | "
                    f"{aggregate.get('variants_passed_all_samples', fallback_variant_passed)} | "
                    f"{aggregate['pass_rate']} | {aggregate['runs']} | "
                    f"{metrics['provenance_coverage']} | {metrics['replay_success']} |"
                )
            _append_metric_axis_table(lines, extra_product)
            _append_system_contract_table(lines, extra_product)
            _append_provenance_overhead_table(lines, extra_product)
            lines.extend(["", "Claim boundary:"])
            if extra_product["paper_evidence"].get("offline_only_notice"):
                lines.append(f"- {extra_product['paper_evidence']['offline_only_notice']}")
            lines.append("")

    if scenario_packs:
        lines.extend(
            [
                "",
                "## Scenario Packs",
                "",
                f"Pack count: {scenario_packs['aggregate']['pack_count']}. Product-eval executable packs: {scenario_packs['aggregate']['executable_product_eval_count']}/{scenario_packs['aggregate']['pack_count']}.",
                "",
                "| Pack | Domain | Status | Tasks | Manifest | Execution |",
                "| --- | --- | --- | ---: | ---: | ---: |",
            ]
        )
        for pack in scenario_packs["packs"]:
            lines.append(
                f"| {pack['pack_id']} | {pack['domain']} | {pack['status']} | {pack['task_count']} | "
                f"{pack['manifest_completeness_score']} | {pack['execution_readiness_score']} |"
            )
        lines.extend(["", "### Scenario-Pack Claim Boundary", ""])
        lines.extend(f"- {claim}" for claim in scenario_packs["paper_claim_boundary"])
    if generated_tasks:
        aggregate = generated_tasks["aggregate"]
        lines.extend(
            [
                "",
                "### Generated Scenario Tasks",
                "",
                f"Seed: `{generated_tasks['seed']}`. Variants: {generated_tasks['variant_count']}. Raw records: {aggregate['raw_evidence_count']}.",
                "",
                "| Metric | Value |",
                "| --- | ---: |",
                f"| Manifest contract pass | {aggregate['manifest_contract_passed']}/{aggregate['runs']} |",
                f"| Product-eval executable variants | {aggregate['product_eval_executable_runs']}/{aggregate['runs']} |",
                f"| Product-eval-backed pending-live-agent variants | {aggregate.get('product_eval_backed_pending_live_agent_runs', 0)} |",
                f"| Runtime-seeded pending-adapter variants | {aggregate.get('runtime_seeded_pending_adapter_runs', 0)} |",
                f"| Manifest-only variants | {aggregate['manifest_only_runs']} |",
                f"| Contract-failed variants | {aggregate['contract_failed_runs']} |",
                "",
                "Generated-task claim boundary:",
            ]
        )
        lines.extend(f"- {claim}" for claim in generated_tasks["paper_claim_boundary"])
    if scenario_loads:
        aggregate = scenario_loads["aggregate"]
        lines.extend(
            [
                "",
                "### Scenario Loader",
                "",
                f"Loaded bundles: {aggregate['pack_count']}. Runtime-seeded bundles: {aggregate['runtime_seeded_count']}/{aggregate['pack_count']}. Manifest-only bundles: {aggregate['manifest_only_count']}.",
                "",
                "| Scenario | Status | Tasks | Runtime Seeded |",
                "| --- | --- | ---: | --- |",
            ]
        )
        for report in scenario_loads["reports"]:
            lines.append(
                f"| {report['scenario_id']} | {report['status']} | {report['task_count']} | "
                f"{report['materialized_runtime']['runtime_seeded']} |"
            )
        lines.extend(["", "Scenario-loader claim boundary:"])
        lines.extend(f"- {claim}" for claim in scenario_loads["paper_claim_boundary"])

    lines.extend(
        [
            "",
            "## Deterministic Benchmark Suite",
            "",
            "| System | Passed | Total | Pass Rate |",
            "| --- | ---: | ---: | ---: |",
        ]
    )
    for system, aggregate in sorted(benchmark["aggregate"].items()):
        lines.append(f"| {system} | {aggregate['passed']} | {aggregate['total']} | {aggregate['pass_rate']} |")
    scale = benchmark["scale_probe"]
    lines.extend(
        [
            "",
            f"Scale probe: {scale['memory_objects_added']} distractors, target retrieved `{scale['target_retrieved']}`, target rank `{scale['target_rank']}`, retrieval time `{scale['retrieval_seconds']}` seconds.",
            "",
            "## Extended Experiments",
            "",
            "| Experiment | Result |",
            "| --- | --- |",
            f"| Noisy retrieval variants | {extended['noisy_retrieval_variants']['passed']}/{extended['noisy_retrieval_variants']['total']} passed |",
            f"| Generated benchmark variants | {extended['generated_benchmark_variants']['passed']}/{extended['generated_benchmark_variants']['total']} passed |",
            f"| Free-form claim extraction corpus size | {extended['free_form_claim_extraction'].get('corpus_size', len(extended['free_form_claim_extraction'].get('cases', [])))} |",
            f"| Free-form claim extraction type precision | {extended['free_form_claim_extraction']['mean_type_precision']} |",
            f"| Free-form claim extraction type recall | {extended['free_form_claim_extraction']['mean_type_recall']} |",
            f"| Free-form claim extraction review recall | {extended['free_form_claim_extraction'].get('mean_review_obligation_recall', 'n/a')} |",
            f"| Verifier engineering valid acceptance | {extended.get('verifier_engineering_benchmark', {}).get('valid_acceptance_rate', 'n/a')} |",
            f"| Verifier engineering invalid rejection | {extended.get('verifier_engineering_benchmark', {}).get('invalid_rejection_rate', 'n/a')} |",
            f"| Security seeded suite | {extended['adversarial_security_suite']['passed']}/{extended['adversarial_security_suite']['total']} passed |",
            f"| Scale retrieval at 5000 distractors | {extended['scale_and_concurrency']['scale_probe']['retrieval_seconds']} seconds |",
            f"| Concurrency p95 latency | {extended['scale_and_concurrency']['concurrency']['p95_latency_seconds']} seconds |",
            f"| Live LLM trials | {extended['live_llm_trials']['status']} |",
            f"| OSS-faithful baseline adapters | {', '.join(extended.get('oss_faithful_baselines', {}).get('adapters', {}).keys()) or 'n/a'} |",
        ]
    )
    if systems_scales:
        lines.extend(
            [
                "",
                "## Indexed Systems-Scale Measurements",
                "",
                "These are descriptive local SQLite/FTS5 measurements on one machine. Repeated operations are not independent task samples.",
                "",
                "| Memory Objects | Provenance Edges | Rank-1 Runs | Serial p50 (s) | Serial p95 (s) | 8-reader p95 (s) | Mixed Errors | DB Bytes |",
                "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
            ]
        )
        for scale_result in sorted(
            systems_scales,
            key=lambda item: int(item.get("memory_scale", {}).get("total_memory_objects", 0)),
        ):
            memory = scale_result["memory_scale"]
            serial = scale_result["serial_retrieval"]
            concurrent = scale_result["concurrent_reads"]
            provenance = scale_result["provenance_growth"]
            mixed = scale_result["mixed_read_write"]
            lines.append(
                f"| {memory['total_memory_objects']} | {provenance['total_edges']} | "
                f"{serial['passed']}/{serial['runs']} | {serial['p50_latency_seconds']} | "
                f"{serial['p95_latency_seconds']} | {concurrent['p95_latency_seconds']} | "
                f"{len(mixed['errors'])} | {memory['database_bytes_after_all_probes']} |"
            )
        lines.extend(
            [
                "",
                "All archived runs kept the FTS index synchronized, observed permission revocation and metric supersession, and completed mixed reads/writes without recorded errors. The million-object latency shows that the local prototype is not yet a low-latency production substrate.",
            ]
        )
    if retrieval_comparisons:
        lines.extend(
            [
                "",
                "## Governed Retrieval-Engine Comparison",
                "",
                "The archived comparison uses internally authored lexical and semantic-paraphrase queries, restricted and superseded near-duplicate traps, and templated distractors. Results are descriptive engine behavior, not independent relevance or product evidence.",
                "",
                "| Distractors | BM25 top-1 | BM25 p95 (s) | MiniLM/HNSW top-1 | MiniLM/HNSW p95 (s) | Hybrid recall@5 | Hybrid p95 (s) | Total leaks |",
                "| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
            ]
        )
        for run in retrieval_comparisons.get("runs", []):
            aggregate = run["aggregate"]
            bm25 = aggregate["bm25_governed"]
            vector = aggregate["minilm_hnsw_governed"]
            hybrid = aggregate["rrf_hybrid_governed"]
            leaks = sum(
                int(metrics["permission_leak_count"]) + int(metrics["superseded_leak_count"])
                for metrics in [bm25, vector, hybrid]
            )
            lines.append(
                f"| {run['distractors']} | {bm25['top1_accuracy']} | {bm25['p95_latency_seconds']} | "
                f"{vector['top1_accuracy']} | {vector['p95_latency_seconds']} | "
                f"{hybrid['recall_at_5']} | {hybrid['p95_latency_seconds']} | {leaks} |"
            )
        lines.extend(
            [
                "",
                "At small scale, the pinned MiniLM/HNSW engine improves semantic-paraphrase retrieval. Under repeated templated semantic neighborhoods, its fixed HNSW settings exhibit crowding and substantial relevance loss by 100k distractors. The hybrid retains broader recall but inherits BM25 full-store fallback latency. Governance metadata updates hide revoked and superseded items in every archived run.",
            ]
        )
    if live_pilot:
        policy = live_pilot.get("policy_trials", {})
        agent = live_pilot.get("live_agent_trials", {})
        policy_intended = policy.get("intended", len(policy.get("trials", [])))
        agent_intended = agent.get("intended", len(agent.get("trials", [])))
        pilot_status = live_pilot.get("status")
        if pilot_status == "completed":
            pilot_boundary = (
                "The corrected-verifier pilot completed every intended task and therefore supports a narrow "
                "end-to-end feasibility claim. It remains one model, three tasks, one prompt per task, and "
                "non-independent grading; it does not support live-model robustness or population performance."
            )
        else:
            pilot_boundary = (
                "The archived pilot is feasibility and failure evidence only. Its incomplete intended denominator "
                "does not support corrected-verifier end-to-end completion or live-model robustness."
            )
        lines.extend(
            [
                "",
                "## Archived Live-Model Feasibility Pilot",
                "",
                f"Status: `{live_pilot.get('status')}`. Provider/model: `{live_pilot.get('provider')}` / `{live_pilot.get('model')}`.",
                "",
                "| Pilot component | Completed | Graded passed |",
                "| --- | ---: | ---: |",
                f"| Policy prompts | {policy.get('completed', 0)}/{policy_intended} | {policy.get('graded_passed', 0)} |",
                f"| End-to-end tasks | {agent.get('completed', 0)}/{agent_intended} | {agent.get('graded_passed', 0)} |",
                f"| Unresolved provider failures | {live_pilot.get('provider_failures', 0)} | n/a |",
                f"| Provider-attempt failures preserved | {live_pilot.get('provider_attempt_failures', live_pilot.get('provider_failures', 0))} | n/a |",
                "",
                pilot_boundary,
            ]
        )
    lines.extend(
        [
            "",
            "## Claims Supported For Paper Draft",
            "",
            "- AMOS runs the payment-failure live-agent vertical slice with retrieval, verifier gates, claim-level provenance, replay, and raw prompt traces.",
            "- In the current local product bundle, AMOS completed all generated payment-failure variants while preserving provenance coverage and replay success.",
            "- The current local bundle includes two policy-aware strong baselines, five simpler baseline families, OSS-faithful adapters (FTS5 RAG corpus, semantic-layer metrics JSON, OpenLineage-shaped events), and raw per-run access-contract metadata.",
            "- Component ablations separately disable verifier enforcement, permission filtering, and claim provenance; the provenance ablation uses a matched provider/tool/replay path.",
            "- Seeded security checks cover permission filtering, prompt-injection-as-evidence behavior, and memory-poisoning demotion.",
            "- Systems measurements cover local indexed retrieval through one million distractors, provenance growth, update visibility, and mixed reads/writes.",
            "- Free-form claim extraction is evaluated on a seed-controlled labeled corpus beyond the original four hand examples.",
        ]
    )
    live = live_pilot or extended.get("live_llm_trials") or {}
    if live.get("status") == "completed":
        live_policy = live.get("policy_trials", {})
        live_agent = live.get("live_agent_trials", {})
        lines.append(
            f"- Live LLM trials completed with provider `{live.get('provider')}` / model `{live.get('model')}` "
            f"(end-to-end graded {live_agent.get('graded_passed', 0)}/{live_agent.get('completed', 0)}; "
            f"policy rubric {live_policy.get('graded_passed', 0)}/{live_policy.get('completed', 0)}; "
            f"preserved provider-attempt failures {live.get('provider_attempt_failures', live.get('provider_failures', 0))})."
        )
    if retrieval_comparisons:
        scales = ", ".join(str(run["distractors"]) for run in retrieval_comparisons.get("runs", []))
        lines.append(
            f"- Governed BM25, pinned MiniLM/HNSW, and reciprocal-rank hybrid retrieval are compared at {scales} "
            "templated distractors, with permission-revocation and supersession probes and all result hashes archived."
        )
    if scenario_packs:
        aggregate = scenario_packs["aggregate"]
        lines.append(
            f"- Versioned scenario manifests cover {aggregate['pack_count']} domains, with {aggregate['executable_product_eval_count']} currently wired into product_eval."
        )
    if extra_products:
        scenarios = ", ".join(f"`{item['scenario']}`" for item in extra_products)
        lines.append(
            f"- Additional live-agent-contract product-eval bundles cover {scenarios} with provider plan/SQL/report phases, raw per-run traces, verifier results, replay, and baseline comparisons."
        )
    if generated_tasks:
        aggregate = generated_tasks["aggregate"]
        lines.append(
            f"- Seeded generated-task evaluation produced {aggregate['runs']} cross-domain scenario variants with {aggregate['manifest_contract_passed']} manifest-contract passes."
        )
    if scenario_loads:
        aggregate = scenario_loads["aggregate"]
        lines.append(
            f"- Scenario loader materialized {aggregate['pack_count']} scenario bundles, including {aggregate['runtime_seeded_count']} runtime-seeded dev fixture."
        )
    lines.extend(
        [
            "",
            "## Claims Not Yet Supported",
            "",
        ]
    )
    if live.get("status") != "completed":
        lines.append(
            f"- Live provider robustness: the current live experiment status is `{live.get('status', 'missing')}`; "
            "provider-backed evidence is incomplete and no robustness claim is supported."
        )
    lines.append(
        "- Hosted enterprise SaaS bakeoffs (Snowflake Cortex, commercial catalog, hosted semantic layers): current OSS-faithful adapters use exported fixtures, not deployed vendor products."
    )
    if retrieval_comparisons:
        lines.append(
            "- Production vector/hybrid superiority: the current comparison uses internal queries, templated distractors, one embedding model, one HNSW configuration, and one machine."
        )
    if scenario_packs:
        aggregate = scenario_packs["aggregate"]
        pending_live = aggregate["pack_count"] - aggregate.get("live_agent_ready_count", 0)
        if pending_live:
            lines.append(
                f"- Cross-domain live-agent coverage: {pending_live} scenario packs still lack a live-agent adapter."
            )
    else:
        lines.append("- Generality beyond payment-failure analysis: scenario packs beyond payment failure are not yet executable in product_eval.")
    lines.extend(
        [
            "- Production security: the attack corpus is seeded and local, not a full red-team assessment.",
            "",
            "## Paper-Ready Tables To Copy",
            "",
            "- Capability-contract aggregate table: see `product_eval/summary.md` or the capability table above.",
            "- Task-family table: `product_eval/family_metrics.csv`.",
            "- Perturbation manifest: `product_eval/variant_manifest.csv`.",
        ]
    )
    if scenario_packs:
        lines.append("- Scenario-pack readiness: `scenario_packs/scenario_pack_summary.md` and `scenario_packs/scenario_pack_coverage.csv`.")
    if generated_tasks:
        lines.append("- Generated scenario tasks: `scenario_packs/generated_tasks_summary.md`, `scenario_packs/generated_tasks.csv`, and `scenario_packs/generated_raw/*.json`.")
    if scenario_loads:
        lines.append("- Scenario loader: `scenario_loads/scenario_load_summary.md` and per-scenario `load_report.json` files.")
    if extra_products:
        lines.append("- Additional capability-contract evaluations: `product_eval_<scenario>/summary.md`, `results.json`, and `raw/*.json`.")
    if systems_scales:
        lines.append("- Indexed systems measurements: `systems_scale*/results.json`, `summary.md`, and `results.sha256`.")
    if retrieval_comparisons:
        lines.append("- Retrieval-engine comparison: `retrieval_engine_comparison/archive_manifest.json` and per-scale `results.json`, `summary.md`, and `results.sha256`.")
    lines.extend(
        [
            "- Failure analysis: `product_eval/failures.md`.",
            "- Systems measurements: `extended_experiments.json` → `scale_and_concurrency`.",
            "",
        ]
    )
    return "\n".join(lines)


def _append_metric_axis_table(lines: list[str], product: dict[str, Any]) -> None:
    lines.extend(
        [
            "",
            "### Metric-Axis Breakdown",
            "",
            "| System | Task | SQL | Metric | Schema | Permission | Provenance | Replay | Review |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for system, aggregate in sorted(product["aggregate"].items()):
        metrics = aggregate["metric_means"]
        lines.append(
            f"| {system} | {metrics.get('task_correctness', 0.0)} | {metrics.get('sql_validity', 0.0)} | "
            f"{metrics.get('metric_correctness', 0.0)} | {metrics.get('schema_correctness', 0.0)} | "
            f"{metrics.get('permission_safety', 0.0)} | {metrics.get('provenance_coverage', 0.0)} | "
            f"{metrics.get('replay_success', 0.0)} | {metrics.get('review_obligation_recall', 0.0)} |"
        )


def _append_system_contract_table(lines: list[str], product: dict[str, Any]) -> None:
    contracts = product.get("system_contracts", {})
    if not contracts:
        return
    lines.extend(
        [
            "",
            "### System Access and Guarantee Contracts",
            "",
            "| System | Category | Current Metric/Schema | Permission Filter | Runtime Verifier | Claim Provenance | Replay |",
            "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for system, contract in sorted(contracts.items()):
        lines.append(
            f"| {system} | {contract.get('category', '')} | {contract.get('current_metric_schema', False)} | "
            f"{contract.get('permission_filter_before_context', False)} | {contract.get('runtime_verifier', False)} | "
            f"{contract.get('claim_provenance', False)} | {contract.get('replay_required', False)} |"
        )


def _append_provenance_overhead_table(lines: list[str], product: dict[str, Any]) -> None:
    overhead = product.get("provenance_overhead", {})
    if not overhead or not overhead.get("pair_count"):
        return
    lines.extend(
        [
            "",
            "### Matched Provenance Overhead",
            "",
            f"Matched executions: {overhead['pair_count']} across {overhead.get('variant_count', 'n/a')} seeded variants. {overhead['design']}.",
            f"{overhead.get('inference_note', '')}",
            "",
            "| Delta (provenance on - off) | Mean | p95 | Seeded-variant bootstrap sensitivity interval |",
            "| --- | ---: | ---: | ---: |",
        ]
    )
    labels = {
        "latency_delta_seconds": "Latency (s)",
        "token_delta": "Tokens",
        "evidence_bytes_delta": "Recorded evidence bytes",
        "replay_delta_seconds": "Replay latency (s)",
    }
    for metric, label in labels.items():
        summary = overhead["summary"][metric]
        ci = summary.get("seeded_variant_bootstrap_interval95") or summary.get("bootstrap_ci95", {"lower": 0.0, "upper": 0.0})
        lines.append(f"| {label} | {summary['mean']} | {summary['p95']} | [{ci['lower']}, {ci['upper']}] |")


def _perturbation_counts(tasks: list[dict[str, Any]]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for task in tasks:
        for perturbation in task.get("perturbations", []):
            counts[perturbation] = counts.get(perturbation, 0) + 1
    return counts


def _rel(root: Path, path: str) -> str:
    target = Path(path)
    try:
        return str(target.resolve().relative_to(root))
    except ValueError:
        return str(target)


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate a consolidated AMOS paper-results report.")
    parser.add_argument("--evaluation-dir", default=None)
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    index = generate_paper_results_report(args.evaluation_dir, args.output)
    print(json.dumps(index, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
