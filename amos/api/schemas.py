from __future__ import annotations

import uuid
from datetime import datetime
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

from amos.memory.models import (
    Authority,
    ArtifactRecord,
    ClaimRecord,
    MemoryObject,
    MemoryStatus,
    MemoryType,
    RunTaskResult,
    VerificationResult,
)


ApiStatus = Literal["pass", "warning", "reject", "needs_review", "error"]


def new_run_id() -> str:
    return f"run_{uuid.uuid4().hex[:12]}"


def api_status(status: str) -> ApiStatus:
    if status == "pass":
        return "pass"
    if status == "warning":
        return "warning"
    if status in {"fail", "reject", "rejected"}:
        return "reject"
    if status == "needs_review":
        return "needs_review"
    return "error"


class ApiContext(BaseModel):
    run_id: str = Field(default_factory=new_run_id)
    user_id: str
    tenant_id: str
    project_id: str


class MemoryRetrieveRequest(BaseModel):
    task_text: str
    required_types: list[MemoryType] = Field(default_factory=list)
    time_range: tuple[datetime, datetime]
    max_items: int = 12
    user_permissions: list[str] | None = Field(
        default=None,
        description="Deprecated. API permissions are resolved from X-AMOS-User.",
    )


class MemoryRetrieveResponse(ApiContext):
    status: ApiStatus
    items: list[MemoryObject]
    filtered_permission_ids: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)
    memory_version_ids: list[str] = Field(default_factory=list)


class MemoryWriteRequest(BaseModel):
    memory_id: str | None = None
    type: MemoryType
    summary: str
    content: dict[str, Any]
    source: str = "api"
    authority: Authority = "user_note"
    effective_start: datetime | None = None
    effective_end: datetime | None = None
    permissions: list[str] | None = None
    sensitivity: str = "internal"
    version: str | None = None
    status: MemoryStatus = "active"
    supersedes: list[str] = Field(default_factory=list)
    provenance_ref: str | None = None


class MemoryWriteResponse(ApiContext):
    status: ApiStatus
    item: MemoryObject | None = None
    memory_version_id: str | None = None
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)


class MemorySupersedeRequest(BaseModel):
    old_memory_id: str
    new_memory_id: str


class MemorySupersedeResponse(ApiContext):
    status: ApiStatus
    old_memory_id: str
    new_memory_id: str
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)


class MemoryReconcileRequest(BaseModel):
    memory_ids: list[str] | None = None


class MemoryReconcileResponse(ApiContext):
    status: ApiStatus
    items: list[MemoryObject]
    warnings: list[str] = Field(default_factory=list)


class TaskRunRequest(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    request: str = Field(alias="user_request")
    provenance_level: int = 3
    permissions: list[str] | None = Field(
        default=None,
        description="Deprecated. API permissions are resolved from X-AMOS-User.",
    )


class TaskRunResponse(ApiContext):
    status: ApiStatus
    result: RunTaskResult | None = None
    task_id: str | None = None
    artifact_id: str | None = None
    replay_package_id: str | None = None
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)


class VerifySqlRequest(BaseModel):
    sql: str
    task_text: str = "payment failure rate investigation"
    time_range: tuple[datetime, datetime] | None = None
    memory_ids: list[str] = Field(default_factory=list)
    max_items: int = 12


class VerifyResponse(ApiContext):
    status: ApiStatus
    verification: VerificationResult | None = None
    passed_checks: list[str] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)
    used_memory_ids: list[str] = Field(default_factory=list)
    filtered_permission_ids: list[str] = Field(default_factory=list)
    provenance_coverage: float = 0.0


class VerifyArtifactRequest(BaseModel):
    artifact_id: str
    provenance_level: int = 3


class ClaimsCiteRequest(BaseModel):
    artifact_id: str
    claim_ids: list[str] | None = None
    provenance_level: int = 3


class ClaimsCiteResponse(ApiContext):
    status: ApiStatus
    artifact_id: str
    claims: list[ClaimRecord]
    citations: list[dict[str, Any]]
    provenance_coverage: float = 0.0
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)


class ArtifactProvenanceResponse(ApiContext):
    status: ApiStatus
    artifact: dict[str, Any]
    claims: list[dict[str, Any]]
    redactions: list[dict[str, Any]] = Field(default_factory=list)
    warnings: list[str] = Field(default_factory=list)


class ArtifactListResponse(ApiContext):
    status: ApiStatus
    artifacts: list[ArtifactRecord]
    warnings: list[str] = Field(default_factory=list)


class ArtifactDetailResponse(ApiContext):
    status: ApiStatus
    artifact: ArtifactRecord | None = None
    report_markdown: str = ""
    claims: list[ClaimRecord] = Field(default_factory=list)
    citations: list[dict[str, Any]] = Field(default_factory=list)
    chart_urls: list[str] = Field(default_factory=list)
    provenance_coverage: float = 0.0
    warnings: list[str] = Field(default_factory=list)


class ReplayApiResponse(ApiContext):
    status: ApiStatus
    artifact_id: str
    replay_status: str
    warnings: list[str] = Field(default_factory=list)
    errors: list[str] = Field(default_factory=list)
