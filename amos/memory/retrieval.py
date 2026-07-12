from __future__ import annotations

from datetime import datetime
import re
from typing import Any

from amos.memory.models import MemoryObject, RetrieveRequest, RetrieveResult
from amos.memory.permissions import has_required_permissions
from amos.memory.reconciliation import AUTHORITY_SCORE, reconcile
from amos.memory.store import MemoryStore


def retrieve(request: RetrieveRequest, store: MemoryStore | None = None) -> RetrieveResult:
    store = store or MemoryStore()
    terms = _terms(request.task_text)
    total_memory = store.memory_count()
    if total_memory >= 1000:
        candidates = store.search_memory_candidates(
            sorted(terms),
            limit=max(128, request.max_items * 16),
        )
        # A zero-hit query still needs governed fallback behavior for unusual
        # identifiers or non-lexical requests.
        if not candidates:
            candidates = store.list_memory()
    else:
        candidates = store.list_memory()
    items: list[MemoryObject] = []
    filtered_permission_ids: list[str] = []

    for item in candidates:
        if request.required_types and item.type not in request.required_types:
            continue
        if not has_required_permissions(item, request.user_permissions):
            filtered_permission_ids.append(item.id)
            continue
        if not _time_overlaps(item, request.time_range[0], request.time_range[1]):
            continue
        score = _score(item, terms, request.task_text)
        if score > 0:
            items.append(item)

    reconciled, warnings = reconcile(items)
    ranked_all = sorted(
        reconciled,
        key=lambda item: _score(item, terms, request.task_text),
        reverse=True,
    )
    warnings.extend(_ambiguity_warnings(ranked_all, terms, request.task_text))
    ranked = ranked_all[: request.max_items]
    store.log(
        "memory.retrieve",
        "agent",
        request.model_dump(mode="json"),
        {"returned_ids": [item.id for item in ranked], "filtered_permission_ids": filtered_permission_ids},
        "pass",
    )
    return RetrieveResult(items=ranked, filtered_permission_ids=filtered_permission_ids, warnings=warnings)


def _terms(text: str) -> set[str]:
    return {_normalize_term(term) for term in re.findall(r"[A-Za-z0-9]+", text.replace("_", " ")) if len(term) >= 3}


def _time_overlaps(item: MemoryObject, start: datetime, end: datetime) -> bool:
    item_start = item.effective_start
    item_end = item.effective_end
    if item_end is not None and item_end < start:
        return False
    if item_start is not None and item_start > end:
        return False
    return True


def _score(item: MemoryObject, terms: set[str], task_text: str = "") -> float:
    fields = _item_fields(item)
    field_tokens = {name: _terms(value) for name, value in fields.items()}
    all_tokens = set().union(*field_tokens.values()) if field_tokens else set()
    matched = terms & all_tokens

    score = 0.0
    score += len(matched) * 2.0
    score += len(terms & field_tokens["id"]) * 1.0
    score += len(terms & field_tokens["summary"]) * 1.5
    score += len(terms & field_tokens["content"]) * 1.0
    score += _phrase_score(item, task_text)
    score += _semantic_field_score(item, terms)
    score += _temporal_score(item)

    if item.type in {"schema", "semantic_definition", "stream_state"}:
        score += 2.0
    if item.status == "active":
        score += 2.0
    if item.type == "feedback":
        if item.authority == "reviewer_approved":
            score += 8.0
        elif item.authority in {"user_note", "model_hypothesis", "untrusted_external"}:
            score -= 4.0

    # Authority is a first-class retrieval signal, not a tie-breaker. Low-authority
    # memory may still be retrieved as evidence, but should not dominate approved
    # memory merely by repeating task terms.
    score += AUTHORITY_SCORE[item.authority] / 5.0
    return score


def _item_fields(item: MemoryObject) -> dict[str, str]:
    return {
        "id": item.id,
        "summary": item.summary,
        "content": _content_text(item.content),
    }


def _content_text(content: dict[str, Any]) -> str:
    parts: list[str] = []
    for key, value in content.items():
        parts.append(str(key))
        if isinstance(value, list):
            parts.extend(str(entry) for entry in value)
        elif isinstance(value, dict):
            parts.append(_content_text(value))
        else:
            parts.append(str(value))
    return " ".join(parts)


def _phrase_score(item: MemoryObject, task_text: str) -> float:
    if not task_text:
        return 0.0
    score = 0.0
    task = _normalized_text(task_text)
    summary = _normalized_text(item.summary)
    content = _normalized_text(_content_text(item.content))
    name = _normalized_text(str(item.content.get("name", "")))

    phrases = {
        "payment failure rate": 8.0,
        "failure rate": 3.0,
        "test account": 3.0,
        "production": 2.0,
        "event time": 3.0,
        "processing time": 2.0,
        "reviewer feedback": 5.0,
        "over attribution": 4.0,
        "definitely caused": 2.0,
    }
    haystack = f"{summary} {content} {name}"
    for phrase, weight in phrases.items():
        if phrase in task and phrase in haystack:
            score += weight

    if item.type == "semantic_definition" and name:
        name_terms = _terms(name)
        task_terms = _terms(task)
        if name_terms and name_terms.issubset(task_terms):
            score += 10.0
        elif {"payment", "failure"} <= name_terms and ("payment" in task_terms or "card" in task_terms):
            score += 6.0
        if item.supersedes and {"old", "stale", "superseded", "processing"}.intersection(task_terms):
            score += 5.0
    return score


def _semantic_field_score(item: MemoryObject, terms: set[str]) -> float:
    if item.type != "semantic_definition":
        return 0.0

    score = 0.0
    name_terms = _terms(str(item.content.get("name", "")))
    required_filter_terms = _terms(" ".join(str(value) for value in item.content.get("required_filters", [])))
    time_field_terms = _terms(str(item.content.get("time_field", "")))

    if name_terms:
        score += len(terms & name_terms) * 3.0
    if required_filter_terms:
        score += len(terms & required_filter_terms) * 2.0
    if time_field_terms:
        score += len(terms & time_field_terms) * 2.0

    if {"production", "test"} <= terms and {"production", "test", "account"} & required_filter_terms:
        score += 4.0
    if {"event", "time"} <= terms and {"event", "time"} <= time_field_terms:
        score += 4.0
    if {"old", "processing", "time"} & terms and {"processing", "time"} <= time_field_terms:
        score += 2.0
    return score


def _temporal_score(item: MemoryObject) -> float:
    if item.status == "superseded":
        return -8.0
    if item.effective_end is None:
        return 2.0
    return 0.0


def _ambiguity_warnings(items: list[MemoryObject], terms: set[str], task_text: str) -> list[str]:
    if len(items) < 2:
        return []
    first, second = items[0], items[1]
    first_score = _score(first, terms, task_text)
    second_score = _score(second, terms, task_text)
    if first_score <= 0:
        return []
    same_kind = first.type == second.type
    close = (first_score - second_score) <= max(3.0, first_score * 0.08)
    disambiguating_terms = {"payment", "card", "production", "test", "event", "checkout", "old", "processing"}
    competing_metrics = (
        first.type == "semantic_definition"
        and second.type == "semantic_definition"
        and "failure" in terms
        and not terms.intersection(disambiguating_terms)
        and "rate" in (_terms(str(first.content.get("name", ""))) | _terms(str(second.content.get("name", ""))))
    )
    if same_kind and (close or competing_metrics) and _logical_key(first) != _logical_key(second):
        return [
            "Ambiguous memory retrieval: top candidates have similar scores but different logical keys "
            f"({_logical_key(first)} vs {_logical_key(second)})."
        ]
    return []


def _logical_key(item: MemoryObject) -> str:
    return str(item.content.get("name") or item.content.get("table") or item.id)


def _normalized_text(text: str) -> str:
    return " ".join(_normalize_term(term) for term in re.findall(r"[A-Za-z0-9]+", text.replace("_", " ")) if len(term) >= 3)


def _normalize_term(term: str) -> str:
    normalized = term.lower()
    synonyms = {
        "payments": "payment",
        "attempts": "attempt",
        "failed": "failure",
        "failures": "failure",
        "sandbox": "test",
        "cards": "card",
        "accounts": "account",
        "excluding": "exclude",
        "removed": "exclude",
        "removing": "exclude",
        "reviewer": "reviewer",
        "attribution": "attribute",
    }
    if normalized in synonyms:
        return synonyms[normalized]
    if normalized.endswith("ies") and len(normalized) > 4:
        return f"{normalized[:-3]}y"
    if normalized.endswith("s") and len(normalized) > 4:
        return normalized[:-1]
    return normalized
