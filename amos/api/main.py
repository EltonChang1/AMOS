from __future__ import annotations

from fastapi import FastAPI

from amos.api import routes_artifacts, routes_claims, routes_memory, routes_replay, routes_tasks, routes_verify


app = FastAPI(title="AMOS Prototype", version="0.1.0")
app.include_router(routes_tasks.router)
app.include_router(routes_memory.router)
app.include_router(routes_verify.router)
app.include_router(routes_claims.router)
app.include_router(routes_artifacts.router)
app.include_router(routes_replay.router)


@app.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}
