from __future__ import annotations

from datetime import datetime
from typing import Any

from pydantic import BaseModel, Field

from amos.memory.models import utc_now


class ClaimProvenance(BaseModel):
    claim_id: str
    claim_text: str
    artifact_id: str
    support: list[str] = Field(default_factory=list)
    query_ids: list[str] = Field(default_factory=list)
    chart_ids: list[str] = Field(default_factory=list)
    document_refs: list[str] = Field(default_factory=list)
    memory_object_ids: list[str] = Field(default_factory=list)
    data_state: dict[str, Any] = Field(default_factory=dict)
    semantic_state: dict[str, Any] = Field(default_factory=dict)
    execution_state: dict[str, Any] = Field(default_factory=dict)
    verification_state: dict[str, Any] = Field(default_factory=dict)
    created_at: datetime = Field(default_factory=utc_now)
