from amos.evaluation.claim_corpus import build_claim_corpus, evaluate_claim_corpus, extract_free_form_claims_v2
from amos.evaluation.oss_faithful_baselines import (
    load_openlineage_events,
    load_rag_documents,
    load_semantic_metrics,
    run_oss_baseline,
    default_payment_sql_builder,
)
from amos.memory.seed_memory import seed_memory
from amos.memory.store import MemoryStore
from amos.tools.seed_duckdb import seed_duckdb


def test_claim_corpus_size_and_metrics():
    corpus = build_claim_corpus(target_size=80, seed=20260711)
    assert len(corpus) == 80
    result = evaluate_claim_corpus(corpus, extract_free_form_claims_v2)
    assert result["mean_type_recall"] >= 0.95
    assert result["mean_type_precision"] >= 0.9
    assert result["mean_review_obligation_recall"] >= 0.95


def test_oss_faithful_fixtures_and_payment_adapters():
    assert len(load_rag_documents()) >= 5
    assert len(load_semantic_metrics()) >= 3
    assert len(load_openlineage_events()) >= 1
    seed_memory(reset=True)
    seed_duckdb()
    store = MemoryStore()
    for adapter in ["oss_rag", "oss_semantic", "oss_catalog"]:
        outcome = run_oss_baseline(
            adapter,
            scenario="payment_failure",
            task_request="Why did payment failure rate increase and should we update the dashboard?",
            task_family="causal",
            expected_evidence=[],
            store=store,
            sql_builder=default_payment_sql_builder(),
        )
        assert outcome.metrics["permission_safety"] is True
        assert outcome.metrics["provenance_coverage"] == 0.0
        assert outcome.metrics["replay_success"] is False
        assert outcome.status in {"pass", "warning", "reject"}
