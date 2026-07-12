from amos.evaluation.verifier_benchmark import run_verifier_benchmark


def test_frozen_verifier_engineering_corpus_has_no_regressions() -> None:
    result = run_verifier_benchmark(write_artifacts=False)
    assert result["total"] >= 16
    assert result["valid_cases"] >= 6
    assert result["invalid_cases"] >= 10
    assert result["false_positive_valid_rejected"] == 0
    assert result["false_negative_invalid_accepted"] == 0
