from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Literal

from pydantic import BaseModel, Field


MemoryType = Literal[
    "active_context",
    "stream_state",
    "schema",
    "semantic_definition",
    "document",
    "prior_analysis",
    "feedback",
    "provenance",
    "permission_policy",
]

Authority = Literal[
    "owner_approved",
    "reviewer_approved",
    "system_observed",
    "user_note",
    "model_hypothesis",
    "untrusted_external",
]

MemoryStatus = Literal["active", "superseded", "rejected", "pending_review"]


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


class User(BaseModel):
    id: str
    permissions: list[str] = Field(default_factory=list)


class MemoryObject(BaseModel):
    id: str
    type: MemoryType
    summary: str
    content: dict[str, Any]
    source: str
    authority: Authority
    effective_start: datetime | None = None
    effective_end: datetime | None = None
    transaction_time: datetime = Field(default_factory=utc_now)
    permissions: list[str] = Field(default_factory=list)
    sensitivity: str = "internal"
    version: str
    status: MemoryStatus = "active"
    supersedes: list[str] = Field(default_factory=list)
    provenance_ref: str | None = None


class RetrieveRequest(BaseModel):
    task_text: str
    required_types: list[MemoryType]
    time_range: tuple[datetime, datetime]
    user_permissions: list[str]
    max_items: int = 12


class RetrieveResult(BaseModel):
    items: list[MemoryObject]
    filtered_permission_ids: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)


class ArtifactRecord(BaseModel):
    artifact_id: str
    artifact_type: Literal["report", "chart", "query", "table", "notebook", "deck"]
    path: str
    user_request: str
    task_plan_id: str
    created_at: datetime = Field(default_factory=utc_now)
    created_by: str
    review_status: Literal["unreviewed", "pending_review", "approved", "rejected"] = "unreviewed"
    provenance_ids: list[str] = Field(default_factory=list)
    replay_package_id: str | None = None


class ClaimRecord(BaseModel):
    claim_id: str
    artifact_id: str
    claim_text: str
    claim_type: Literal["numeric", "causal", "recommendation", "context"]
    requires_review: bool = False


class ReplayPackage(BaseModel):
    replay_package_id: str
    artifact_id: str
    user_request: str
    task_plan: dict[str, Any]
    query_ids: list[str]
    chart_ids: list[str]
    memory_snapshot_ids: list[str]
    schema_versions: list[str]
    semantic_definition_versions: list[str]
    stream_or_snapshot_state: dict[str, Any]
    tool_versions: dict[str, str]
    verification_report_id: str
    created_at: datetime = Field(default_factory=utc_now)


class VerificationResult(BaseModel):
    status: Literal["pass", "warning", "fail"]
    passed_checks: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)
    provenance_coverage: float = 0.0


class RunTaskResult(BaseModel):
    task_id: str
    artifact_id: str
    report_path: str
    chart_paths: list[str]
    verification_status: Literal["pass", "warning", "fail"]
    warnings: list[str]
    provenance_ids: list[str]
    replay_package_id: str
    used_memory_ids: list[str]
    provenance_coverage: float
