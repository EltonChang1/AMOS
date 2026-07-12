from __future__ import annotations

from amos.memory.models import MemoryObject
from amos.memory.permissions import has_required_permissions


def check_memory_permissions(items: list[MemoryObject], user_permissions: list[str]) -> tuple[list[str], list[str]]:
    errors = [
        f"Memory item {item.id} requires permissions {item.permissions}."
        for item in items
        if not has_required_permissions(item, user_permissions)
    ]
    return [], errors
