from __future__ import annotations

from typing import Annotated

from fastapi import Header, HTTPException
from pydantic import BaseModel, Field

from amos.memory.models import User


class Identity(BaseModel):
    user_id: str
    roles: list[str] = Field(default_factory=list)
    permissions: list[str] = Field(default_factory=list)
    tenant_id: str = "tenant_default"
    project_id: str = "payments"

    def as_user(self) -> User:
        return User(id=self.user_id, permissions=self.permissions)


DEV_IDENTITIES: dict[str, Identity] = {
    "analyst_001": Identity(
        user_id="analyst_001",
        roles=["analyst"],
        permissions=["analytics", "payments"],
    ),
    "reviewer_001": Identity(
        user_id="reviewer_001",
        roles=["reviewer"],
        permissions=["analytics", "payments"],
    ),
    "sre_001": Identity(
        user_id="sre_001",
        roles=["analyst", "sre"],
        permissions=["analytics", "payments", "sre"],
    ),
    "admin": Identity(
        user_id="admin",
        roles=["admin"],
        permissions=["analytics", "payments", "sre", "admin"],
    ),
}


def get_identity(x_amos_user: Annotated[str | None, Header(alias="X-AMOS-User")] = None) -> Identity:
    user_id = x_amos_user or "analyst_001"
    identity = DEV_IDENTITIES.get(user_id)
    if identity is None:
        raise HTTPException(status_code=401, detail=f"Unknown AMOS dev user: {user_id}")
    return identity


def api_context(identity: Identity, run_id: str) -> dict[str, str]:
    return {
        "run_id": run_id,
        "user_id": identity.user_id,
        "tenant_id": identity.tenant_id,
        "project_id": identity.project_id,
    }
