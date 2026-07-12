"""Seed-controlled free-form claim extraction corpus.

Scales the four-example probe to dozens of labeled artifacts across report,
notebook, slide, ambiguous, subscription, and warehouse domains. Labels are
stored with the corpus (not inferred at scoring time).
"""

from __future__ import annotations

import json
import random
import re
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Callable

from amos.config import settings


@dataclass(frozen=True)
class ClaimExample:
    example_id: str
    domain: str
    artifact_kind: str
    text: str
    expected_types: list[str]
    expected_review_claims: int


ExtractFn = Callable[[str, str], list[dict[str, Any]]]


def build_claim_corpus(*, target_size: int = 80, seed: int = 20260711) -> list[ClaimExample]:
    rng = random.Random(seed)
    core = _hand_authored_core()
    generated: list[ClaimExample] = []
    domains = [
        ("payment_report", "report", _payment_report_template),
        ("payment_notebook", "notebook", _payment_notebook_template),
        ("payment_slide", "slide", _payment_slide_template),
        ("payment_ambiguous", "ambiguous", _payment_ambiguous_template),
        ("subscription", "subscription_churn", _subscription_template),
        ("warehouse", "warehouse_quality", _warehouse_template),
    ]
    index = 0
    while len(core) + len(generated) < max(target_size, len(core)):
        domain_name, kind, builder = domains[index % len(domains)]
        example = builder(rng, index // len(domains), domain_name, kind)
        generated.append(example)
        index += 1
    corpus = [*core, *generated]
    return corpus[: max(target_size, len(core))]


def evaluate_claim_corpus(
    corpus: list[ClaimExample],
    extractor: ExtractFn,
) -> dict[str, Any]:
    rows = []
    for example in corpus:
        extracted = extractor(example.text, example.example_id)
        extracted_types = [claim["claim_type"] for claim in extracted]
        observed_review = sum(1 for claim in extracted if claim.get("requires_review"))
        expected_review = int(example.expected_review_claims)
        rows.append(
            {
                "example_id": example.example_id,
                "domain": example.domain,
                "artifact_kind": example.artifact_kind,
                "expected_types": list(example.expected_types),
                "extracted_types": extracted_types,
                "type_recall": _multiset_recall(example.expected_types, extracted_types),
                "type_precision": _multiset_precision(example.expected_types, extracted_types),
                "expected_review_claims": expected_review,
                "requires_review_claims": observed_review,
                "review_obligation_recall": (
                    round(min(observed_review, expected_review) / expected_review, 3)
                    if expected_review
                    else 1.0
                ),
            }
        )

    def _mean(key: str) -> float:
        return round(sum(row[key] for row in rows) / len(rows), 3) if rows else 0.0

    by_domain: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        by_domain.setdefault(row["domain"], []).append(row)
    domain_summary = {
        domain: {
            "n": len(domain_rows),
            "mean_type_recall": round(sum(r["type_recall"] for r in domain_rows) / len(domain_rows), 3),
            "mean_type_precision": round(sum(r["type_precision"] for r in domain_rows) / len(domain_rows), 3),
            "mean_review_obligation_recall": round(
                sum(r["review_obligation_recall"] for r in domain_rows) / len(domain_rows), 3
            ),
        }
        for domain, domain_rows in sorted(by_domain.items())
    }
    return {
        "description": (
            "Seed-controlled free-form claim corpus evaluated with a regex/keyword extractor. "
            "This scales the probe; it is not production NLP evidence."
        ),
        "extractor": "regex_and_keyword_claim_type_classifier_v2",
        "corpus_size": len(corpus),
        "seed": None,
        "cases": rows,
        "mean_type_recall": _mean("type_recall"),
        "mean_type_precision": _mean("type_precision"),
        "mean_review_obligation_recall": _mean("review_obligation_recall"),
        "domain_summary": domain_summary,
        "limitation": (
            "Labels are synthetic and the extractor is regex/keyword based. "
            "Do not claim robust free-form claim extraction for arbitrary analyst prose."
        ),
    }


def write_claim_corpus_artifacts(result: dict[str, Any], corpus: list[ClaimExample], seed: int) -> dict[str, str]:
    out_dir = settings.artifact_dir / "evaluation" / "claim_extraction"
    out_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = out_dir / "corpus.json"
    results_path = out_dir / "corpus_results.json"
    summary_path = out_dir / "summary.md"
    payload = {**result, "seed": seed}
    corpus_path.write_text(
        json.dumps([asdict(example) for example in corpus], indent=2, sort_keys=True),
        encoding="utf-8",
    )
    results_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
    lines = [
        "# Claim Extraction Corpus Results",
        "",
        f"- Corpus size: {payload['corpus_size']}",
        f"- Seed: {seed}",
        f"- Mean type precision: {payload['mean_type_precision']}",
        f"- Mean type recall: {payload['mean_type_recall']}",
        f"- Mean review-obligation recall: {payload['mean_review_obligation_recall']}",
        "",
        "## Domain Summary",
        "",
        "| Domain | N | Type P | Type R | Review recall |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for domain, stats in payload["domain_summary"].items():
        lines.append(
            f"| {domain} | {stats['n']} | {stats['mean_type_precision']} | "
            f"{stats['mean_type_recall']} | {stats['mean_review_obligation_recall']} |"
        )
    lines.extend(["", f"Limitation: {payload['limitation']}", ""])
    summary_path.write_text("\n".join(lines), encoding="utf-8")
    return {
        "corpus": str(corpus_path),
        "results": str(results_path),
        "summary": str(summary_path),
    }


def extract_free_form_claims_v2(text: str, artifact_id: str) -> list[dict[str, Any]]:
    """Improved regex/keyword extractor with negation and bullet handling."""
    sentences = [
        segment.strip(" -\n\t•*")
        for segment in re.split(r"(?<=[.!?])\s+|\n+", text)
        if segment.strip(" -\n\t•*")
    ]
    claims: list[dict[str, Any]] = []
    for index, sentence in enumerate(sentences):
        lower = sentence.lower()
        claim_type: str | None = None
        requires_review = False
        hedged_causal = any(
            term in lower
            for term in [
                "may have",
                "might have",
                "could have",
                "suspect",
                "not enough to say",
                "cannot conclude",
                "do not claim",
                "needs human review",
                "requires review",
                "pending-review",
                "pending review",
            ]
        )
        numeric_hit = any(
            term in lower for term in ["%", "increased", "rose", "reached", "grew", "dropped", "fell", "rate"]
        ) or bool(re.search(r"\d+\.\d+", sentence))
        causal_hit = any(
            term in lower for term in ["caused", "contributed", "because", "due to", "causality", "root cause"]
        ) or hedged_causal
        reco_hit = any(
            term in lower
            for term in [
                "should",
                "recommend",
                "recommendation",
                "dashboard update",
                "annotated",
                "keep",
                "escalate",
            ]
        )
        if numeric_hit and not causal_hit and not reco_hit:
            claim_type = "numeric"
        if causal_hit:
            claim_type = "causal"
            requires_review = True
        if reco_hit and not causal_hit:
            claim_type = "recommendation"
            requires_review = True
        if reco_hit and causal_hit:
            # Prefer causal when both present; recommendation often co-occurs in review text.
            claim_type = "causal"
            requires_review = True
        if "recommend" in lower or "dashboard" in lower:
            if claim_type != "causal":
                claim_type = "recommendation"
                requires_review = True
            elif "recommend" in lower:
                # Emit an additional recommendation claim for explicit recommend language.
                claims.append(
                    {
                        "claim_id": f"claim_{artifact_id}_{index}_reco",
                        "claim_text": sentence,
                        "claim_type": "recommendation",
                        "requires_review": True,
                    }
                )
        if claim_type:
            claims.append(
                {
                    "claim_id": f"claim_{artifact_id}_{index}",
                    "claim_text": sentence,
                    "claim_type": claim_type,
                    "requires_review": requires_review,
                }
            )
    return claims


def _hand_authored_core() -> list[ClaimExample]:
    return [
        ClaimExample(
            example_id="core_free_form_report",
            domain="payment_report",
            artifact_kind="report",
            text=(
                "Payment failure rate rose from 2.2% to 7.4% in the current six-hour window. "
                "The highest concentration was Processor B / Visa at 15.8%. The deployment may "
                "have contributed, but this needs human review. The dashboard should be annotated, not finalized."
            ),
            expected_types=["numeric", "numeric", "causal", "recommendation"],
            expected_review_claims=2,
        ),
        ClaimExample(
            example_id="core_notebook_markdown",
            domain="payment_notebook",
            artifact_kind="notebook",
            text=(
                "The current window has 7.4% failures versus 2.2% previously. I suspect the gateway "
                "release caused the spike. Recommendation: keep the executive dashboard in pending-review state."
            ),
            expected_types=["numeric", "causal", "recommendation"],
            expected_review_claims=2,
        ),
        ClaimExample(
            example_id="core_slide_bullets",
            domain="payment_slide",
            artifact_kind="slide",
            text=(
                "- Failures increased to 7.4%.\n"
                "- Processor B / Visa reached 15.8%.\n"
                "- Do not claim causality until a causal design is approved.\n"
                "- Recommend review before dashboard update."
            ),
            expected_types=["numeric", "numeric", "causal", "recommendation"],
            expected_review_claims=2,
        ),
        ClaimExample(
            example_id="core_ambiguous_prose",
            domain="payment_ambiguous",
            artifact_kind="ambiguous",
            text=(
                "Payments looked worse after the release, especially around Processor B. There is enough "
                "evidence to investigate, but not enough to say the release caused it."
            ),
            expected_types=["causal"],
            expected_review_claims=1,
        ),
    ]


def _payment_report_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    prev = round(rng.uniform(1.5, 3.0), 1)
    curr = round(rng.uniform(5.0, 12.0), 1)
    conc = round(rng.uniform(10.0, 20.0), 1)
    processor = rng.choice(["Processor B", "Processor C", "Processor A"])
    network = rng.choice(["Visa", "Mastercard", "Amex"])
    text = (
        f"Payment failure rate rose from {prev}% to {curr}% in the current window. "
        f"{processor} / {network} reached {conc}%. "
        f"The deployment may have contributed and needs human review. "
        f"Recommend keeping the dashboard annotated until review completes."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["numeric", "numeric", "causal", "recommendation"],
        expected_review_claims=2,
    )


def _payment_notebook_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    curr = round(rng.uniform(5.0, 11.0), 1)
    prev = round(rng.uniform(1.5, 3.5), 1)
    text = (
        f"Notebook note: failures are {curr}% versus {prev}% previously. "
        f"I suspect the release caused the spike. "
        f"Recommendation: keep executive reporting in pending-review state."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["numeric", "causal", "recommendation"],
        expected_review_claims=2,
    )


def _payment_slide_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    rate = round(rng.uniform(6.0, 10.0), 1)
    conc = round(rng.uniform(12.0, 18.0), 1)
    text = (
        f"- Failures increased to {rate}%.\n"
        f"- Concentration reached {conc}%.\n"
        f"- Do not claim causality without an approved design.\n"
        f"- Recommend review before dashboard update."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["numeric", "numeric", "causal", "recommendation"],
        expected_review_claims=2,
    )


def _payment_ambiguous_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    actor = rng.choice(["the release", "the gateway deploy", "the processor change"])
    text = (
        f"Payments looked worse after {actor}. There is enough signal to investigate, "
        f"but not enough to say {actor} caused it."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["causal"],
        expected_review_claims=1,
    )


def _subscription_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    churn = round(rng.uniform(3.0, 9.0), 1)
    prev = round(rng.uniform(1.0, 2.5), 1)
    text = (
        f"Subscription churn rose from {prev}% to {churn}% this week. "
        f"Billing plan changes may have contributed and need human review. "
        f"Recommend holding the executive churn dashboard update."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["numeric", "causal", "recommendation"],
        expected_review_claims=2,
    )


def _warehouse_template(rng: random.Random, idx: int, domain: str, kind: str) -> ClaimExample:
    acc = round(rng.uniform(88.0, 96.0), 1)
    prev = round(rng.uniform(96.0, 99.0), 1)
    text = (
        f"Pick accuracy dropped from {prev}% to {acc}%. "
        f"SKU remap may have contributed; this needs human review. "
        f"Recommend annotating the warehouse quality board."
    )
    return ClaimExample(
        example_id=f"gen_{domain}_{idx:03d}",
        domain=domain,
        artifact_kind=kind,
        text=text,
        expected_types=["numeric", "causal", "recommendation"],
        expected_review_claims=2,
    )


def _multiset_recall(expected: list[str], observed: list[str]) -> float:
    if not expected:
        return 1.0
    remaining = list(observed)
    hits = 0
    for item in expected:
        if item in remaining:
            remaining.remove(item)
            hits += 1
    return round(hits / len(expected), 3)


def _multiset_precision(expected: list[str], observed: list[str]) -> float:
    if not observed:
        return 1.0 if not expected else 0.0
    remaining = list(expected)
    hits = 0
    for item in observed:
        if item in remaining:
            remaining.remove(item)
            hits += 1
    return round(hits / len(observed), 3)
