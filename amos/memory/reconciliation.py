from __future__ import annotations

from collections import defaultdict

from amos.memory.models import MemoryObject


AUTHORITY_SCORE = {
    "owner_approved": 60,
    "reviewer_approved": 50,
    "system_observed": 40,
    "user_note": 30,
    "model_hypothesis": 20,
    "untrusted_external": 10,
}


def reconcile(items: list[MemoryObject]) -> tuple[list[MemoryObject], list[str]]:
    warnings: list[str] = []
    active = [item for item in items if item.status == "active"]
    superseded_ids = {old for item in active for old in item.supersedes}
    active = [item for item in active if item.id not in superseded_ids]

    by_kind: dict[tuple[str, str], list[MemoryObject]] = defaultdict(list)
    for item in active:
        logical_key = item.content.get("name") or item.content.get("table") or item.id
        by_kind[(item.type, str(logical_key))].append(item)

    selected: list[MemoryObject] = []
    for (_type, logical_key), group in by_kind.items():
        owner_approved = [item for item in group if item.authority == "owner_approved"]
        if len(owner_approved) > 1:
            versions = ", ".join(sorted(item.id for item in owner_approved))
            warnings.append(f"Conflicting owner-approved memory for {logical_key}: {versions}")
        selected.extend(sorted(group, key=_rank, reverse=True)[:1])

    return sorted(selected, key=_rank, reverse=True), warnings


def _rank(item: MemoryObject) -> tuple[int, int, str]:
    status_score = 1 if item.status == "active" else 0
    return (AUTHORITY_SCORE[item.authority], status_score, item.transaction_time.isoformat())
