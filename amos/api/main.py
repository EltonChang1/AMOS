from __future__ import annotations

from pathlib import Path

from fastapi import FastAPI
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from amos.api import routes_artifacts, routes_claims, routes_memory, routes_replay, routes_tasks, routes_verify


WEB_DIR = Path(__file__).with_name("web")


app = FastAPI(
    title="AMOS",
    version="0.1.0",
    description="Verified, replayable analysis for high-stakes data decisions.",
)
app.include_router(routes_tasks.router)
app.include_router(routes_memory.router)
app.include_router(routes_verify.router)
app.include_router(routes_claims.router)
app.include_router(routes_artifacts.router)
app.include_router(routes_replay.router)
app.mount("/static", StaticFiles(directory=WEB_DIR), name="static")


@app.get("/", include_in_schema=False, response_class=FileResponse)
def product_home() -> FileResponse:
    return FileResponse(WEB_DIR / "index.html")


@app.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}
