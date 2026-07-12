from __future__ import annotations

from amos.memory.models import MemoryObject


def has_required_permissions(item: MemoryObject, user_permissions: list[str]) -> bool:
    required = set(item.permissions)
    granted = set(user_permissions)
    return required.issubset(granted)


def redact_memory(item: MemoryObject) -> dict[str, object]:
    return {
        "id": item.id,
        "type": item.type,
        "summary": item.summary,
        "sensitivity": item.sensitivity,
        "redacted": True,
    }
