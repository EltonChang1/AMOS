from __future__ import annotations

import json
import sqlite3
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

from amos.config import settings
from amos.memory.models import ArtifactRecord, ClaimRecord, MemoryObject, ReplayPackage
from amos.provenance.models import ClaimProvenance


def _dt(value: datetime | None) -> str | None:
    return value.isoformat() if value else None


def _parse_dt(value: str | None) -> datetime | None:
    if not value:
        return None
    parsed = datetime.fromisoformat(value)
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=timezone.utc)
    return parsed


class MemoryStore:
    def __init__(self, db_path: Path | None = None) -> None:
        self.db_path = db_path or settings.memory_db
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._schema_initialized = False

    def connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.db_path, timeout=30.0)
        conn.row_factory = sqlite3.Row
        return conn

    def init_schema(self) -> None:
        if self._schema_initialized and self.db_path.exists():
            return
        with self.connect() as conn:
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
            conn.executescript(
                """
                CREATE TABLE IF NOT EXISTS memory_objects (
                    id TEXT PRIMARY KEY,
                    type TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    content_json TEXT NOT NULL,
                    source TEXT NOT NULL,
                    authority TEXT NOT NULL,
                    effective_start TEXT,
                    effective_end TEXT,
                    transaction_time TEXT NOT NULL,
                    permissions_json TEXT NOT NULL,
                    sensitivity TEXT NOT NULL,
                    version TEXT NOT NULL,
                    status TEXT NOT NULL,
                    supersedes_json TEXT NOT NULL,
                    provenance_ref TEXT
                );

                CREATE TABLE IF NOT EXISTS artifacts (
                    artifact_id TEXT PRIMARY KEY,
                    artifact_type TEXT NOT NULL,
                    path TEXT NOT NULL,
                    user_request TEXT NOT NULL,
                    task_plan_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    created_by TEXT NOT NULL,
                    review_status TEXT NOT NULL,
                    provenance_ids_json TEXT NOT NULL,
                    replay_package_id TEXT
                );

                CREATE TABLE IF NOT EXISTS claims (
                    claim_id TEXT PRIMARY KEY,
                    artifact_id TEXT NOT NULL,
                    claim_text TEXT NOT NULL,
                    claim_type TEXT NOT NULL,
                    requires_review INTEGER NOT NULL,
                    FOREIGN KEY (artifact_id) REFERENCES artifacts(artifact_id)
                );

                CREATE TABLE IF NOT EXISTS claim_provenance (
                    claim_id TEXT PRIMARY KEY,
                    artifact_id TEXT NOT NULL,
                    provenance_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS provenance_edges (
                    edge_id TEXT PRIMARY KEY,
                    source_id TEXT NOT NULL,
                    target_id TEXT NOT NULL,
                    relation TEXT NOT NULL,
                    metadata_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS replay_packages (
                    replay_package_id TEXT PRIMARY KEY,
                    artifact_id TEXT NOT NULL,
                    package_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (artifact_id) REFERENCES artifacts(artifact_id)
                );

                CREATE TABLE IF NOT EXISTS audit_log (
                    audit_id TEXT PRIMARY KEY,
                    operation TEXT NOT NULL,
                    actor TEXT NOT NULL,
                    task_id TEXT,
                    input_json TEXT NOT NULL,
                    output_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                    id UNINDEXED,
                    type UNINDEXED,
                    summary,
                    content_json,
                    content='memory_objects',
                    content_rowid='rowid'
                );

                CREATE TRIGGER IF NOT EXISTS memory_objects_ai AFTER INSERT ON memory_objects BEGIN
                    INSERT INTO memory_fts(rowid, id, type, summary, content_json)
                    VALUES (new.rowid, new.id, new.type, new.summary, new.content_json);
                END;

                CREATE TRIGGER IF NOT EXISTS memory_objects_ad AFTER DELETE ON memory_objects BEGIN
                    INSERT INTO memory_fts(memory_fts, rowid, id, type, summary, content_json)
                    VALUES ('delete', old.rowid, old.id, old.type, old.summary, old.content_json);
                END;

                CREATE TRIGGER IF NOT EXISTS memory_objects_au AFTER UPDATE ON memory_objects BEGIN
                    INSERT INTO memory_fts(memory_fts, rowid, id, type, summary, content_json)
                    VALUES ('delete', old.rowid, old.id, old.type, old.summary, old.content_json);
                    INSERT INTO memory_fts(rowid, id, type, summary, content_json)
                    VALUES (new.rowid, new.id, new.type, new.summary, new.content_json);
                END;

                CREATE INDEX IF NOT EXISTS idx_memory_type_status
                ON memory_objects(type, status);

                CREATE INDEX IF NOT EXISTS idx_provenance_source
                ON provenance_edges(source_id);

                CREATE INDEX IF NOT EXISTS idx_provenance_target
                ON provenance_edges(target_id);
                """
            )
            memory_count = int(conn.execute("SELECT COUNT(*) FROM memory_objects").fetchone()[0])
            fts_count = int(conn.execute("SELECT COUNT(*) FROM memory_fts").fetchone()[0])
            if memory_count != fts_count:
                conn.execute("INSERT INTO memory_fts(memory_fts) VALUES ('rebuild')")
        self._schema_initialized = True

    def reset(self) -> None:
        self._schema_initialized = False
        if self.db_path.exists():
            self.db_path.unlink()
        wal_path = self.db_path.with_name(f"{self.db_path.name}-wal")
        shm_path = self.db_path.with_name(f"{self.db_path.name}-shm")
        wal_path.unlink(missing_ok=True)
        shm_path.unlink(missing_ok=True)
        self.init_schema()

    def upsert_memory(self, item: MemoryObject) -> None:
        self.init_schema()
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO memory_objects
                (id, type, summary, content_json, source, authority, effective_start,
                 effective_end, transaction_time, permissions_json, sensitivity, version,
                 status, supersedes_json, provenance_ref)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                self._memory_values(item),
            )
        self.log("memory.write", "system", {"id": item.id}, {"status": item.status}, "pass")

    def bulk_upsert_memory(self, items: Iterable[MemoryObject], *, batch_size: int = 1000) -> int:
        """Insert memory objects in bounded transactions with one audit record.

        This is intended for imports and scale experiments. FTS triggers keep the
        candidate index synchronized; individual synthetic objects do not create
        one audit row each.
        """

        self.init_schema()
        sql = """
            INSERT OR REPLACE INTO memory_objects
            (id, type, summary, content_json, source, authority, effective_start,
             effective_end, transaction_time, permissions_json, sensitivity, version,
             status, supersedes_json, provenance_ref)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """
        count = 0
        batch: list[tuple[Any, ...]] = []
        with self.connect() as conn:
            for item in items:
                batch.append(self._memory_values(item))
                if len(batch) >= batch_size:
                    conn.executemany(sql, batch)
                    count += len(batch)
                    batch.clear()
            if batch:
                conn.executemany(sql, batch)
                count += len(batch)
        self.log("memory.bulk_write", "system", {"count": count}, {"status": "active"}, "pass")
        return count

    def memory_count(self) -> int:
        self.init_schema()
        with self.connect() as conn:
            return int(conn.execute("SELECT COUNT(*) FROM memory_objects").fetchone()[0])

    def search_memory_candidates(self, terms: list[str], *, limit: int = 512) -> list[MemoryObject]:
        """Return FTS5 candidates without exposing content to the model.

        Governance, temporal checks, authority ranking, and permission filtering
        still run in the retrieval layer after candidate generation.
        """

        cleaned = ["".join(char for char in term.lower() if char.isalnum()) for term in terms]
        cleaned = [term for term in cleaned if term]
        if not cleaned:
            return []
        query = " OR ".join(f'"{term}"' for term in sorted(set(cleaned)))
        self.init_schema()
        with self.connect() as conn:
            rows = conn.execute(
                """
                SELECT m.*
                FROM memory_fts
                JOIN memory_objects AS m ON m.rowid = memory_fts.rowid
                WHERE memory_fts MATCH ?
                ORDER BY bm25(memory_fts)
                LIMIT ?
                """,
                (query, max(limit, 1)),
            ).fetchall()
        return [self._row_to_memory(row) for row in rows]

    def get_memory(self, memory_id: str) -> MemoryObject | None:
        self.init_schema()
        with self.connect() as conn:
            row = conn.execute("SELECT * FROM memory_objects WHERE id = ?", (memory_id,)).fetchone()
        return self._row_to_memory(row) if row else None

    def list_memory(self) -> list[MemoryObject]:
        self.init_schema()
        with self.connect() as conn:
            rows = conn.execute("SELECT * FROM memory_objects").fetchall()
        return [self._row_to_memory(row) for row in rows]

    @staticmethod
    def _memory_values(item: MemoryObject) -> tuple[Any, ...]:
        return (
            item.id,
            item.type,
            item.summary,
            json.dumps(item.content, sort_keys=True),
            item.source,
            item.authority,
            _dt(item.effective_start),
            _dt(item.effective_end),
            _dt(item.transaction_time),
            json.dumps(item.permissions),
            item.sensitivity,
            item.version,
            item.status,
            json.dumps(item.supersedes),
            item.provenance_ref,
        )

    def supersede(self, old_id: str, new_id: str) -> None:
        with self.connect() as conn:
            conn.execute("UPDATE memory_objects SET status = 'superseded' WHERE id = ?", (old_id,))
            row = conn.execute("SELECT supersedes_json FROM memory_objects WHERE id = ?", (new_id,)).fetchone()
            supersedes = json.loads(row["supersedes_json"]) if row else []
            if old_id not in supersedes:
                supersedes.append(old_id)
            conn.execute(
                "UPDATE memory_objects SET supersedes_json = ? WHERE id = ?",
                (json.dumps(supersedes), new_id),
            )
        self.log("memory.supersede", "system", {"old_id": old_id, "new_id": new_id}, {}, "pass")

    def add_artifact(self, artifact: ArtifactRecord) -> None:
        self.init_schema()
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO artifacts
                (artifact_id, artifact_type, path, user_request, task_plan_id, created_at,
                 created_by, review_status, provenance_ids_json, replay_package_id)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    artifact.artifact_id,
                    artifact.artifact_type,
                    artifact.path,
                    artifact.user_request,
                    artifact.task_plan_id,
                    _dt(artifact.created_at),
                    artifact.created_by,
                    artifact.review_status,
                    json.dumps(artifact.provenance_ids),
                    artifact.replay_package_id,
                ),
            )

    def update_artifact_provenance(self, artifact_id: str, provenance_ids: list[str], replay_package_id: str) -> None:
        with self.connect() as conn:
            conn.execute(
                """
                UPDATE artifacts
                SET provenance_ids_json = ?, replay_package_id = ?
                WHERE artifact_id = ?
                """,
                (json.dumps(provenance_ids), replay_package_id, artifact_id),
            )

    def get_artifact(self, artifact_id: str) -> ArtifactRecord | None:
        self.init_schema()
        with self.connect() as conn:
            row = conn.execute("SELECT * FROM artifacts WHERE artifact_id = ?", (artifact_id,)).fetchone()
        if not row:
            return None
        return ArtifactRecord(
            artifact_id=row["artifact_id"],
            artifact_type=row["artifact_type"],
            path=row["path"],
            user_request=row["user_request"],
            task_plan_id=row["task_plan_id"],
            created_at=_parse_dt(row["created_at"]),
            created_by=row["created_by"],
            review_status=row["review_status"],
            provenance_ids=json.loads(row["provenance_ids_json"]),
            replay_package_id=row["replay_package_id"],
        )

    def add_claim(self, claim: ClaimRecord) -> None:
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO claims
                (claim_id, artifact_id, claim_text, claim_type, requires_review)
                VALUES (?, ?, ?, ?, ?)
                """,
                (
                    claim.claim_id,
                    claim.artifact_id,
                    claim.claim_text,
                    claim.claim_type,
                    int(claim.requires_review),
                ),
            )

    def list_claims(self, artifact_id: str) -> list[ClaimRecord]:
        with self.connect() as conn:
            rows = conn.execute("SELECT * FROM claims WHERE artifact_id = ?", (artifact_id,)).fetchall()
        return [
            ClaimRecord(
                claim_id=row["claim_id"],
                artifact_id=row["artifact_id"],
                claim_text=row["claim_text"],
                claim_type=row["claim_type"],
                requires_review=bool(row["requires_review"]),
            )
            for row in rows
        ]

    def add_claim_provenance(self, provenance: ClaimProvenance) -> None:
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO claim_provenance
                (claim_id, artifact_id, provenance_json, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (
                    provenance.claim_id,
                    provenance.artifact_id,
                    provenance.model_dump_json(),
                    _dt(provenance.created_at),
                ),
            )
        for target in provenance.support:
            self.add_edge(provenance.claim_id, target, "supported_by", {})

    def list_claim_provenance(self, artifact_id: str) -> list[ClaimProvenance]:
        with self.connect() as conn:
            rows = conn.execute(
                "SELECT provenance_json FROM claim_provenance WHERE artifact_id = ?",
                (artifact_id,),
            ).fetchall()
        return [ClaimProvenance.model_validate_json(row["provenance_json"]) for row in rows]

    def add_edge(self, source_id: str, target_id: str, relation: str, metadata: dict[str, Any]) -> None:
        self.init_schema()
        with self.connect() as conn:
            conn.execute(
                """
                INSERT INTO provenance_edges
                (edge_id, source_id, target_id, relation, metadata_json, created_at)
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    f"edge_{uuid.uuid4().hex}",
                    source_id,
                    target_id,
                    relation,
                    json.dumps(metadata, sort_keys=True),
                    datetime.now(timezone.utc).isoformat(),
                ),
            )

    def bulk_add_edges(
        self,
        edges: Iterable[tuple[str, str, str, dict[str, Any]]],
        *,
        batch_size: int = 2000,
    ) -> int:
        """Insert provenance edges in bounded transactions for imports and benchmarks."""

        self.init_schema()
        sql = """
            INSERT INTO provenance_edges
            (edge_id, source_id, target_id, relation, metadata_json, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
        """
        count = 0
        batch: list[tuple[str, str, str, str, str, str]] = []
        created_at = datetime.now(timezone.utc).isoformat()
        with self.connect() as conn:
            for source_id, target_id, relation, metadata in edges:
                batch.append(
                    (
                        f"edge_{uuid.uuid4().hex}",
                        source_id,
                        target_id,
                        relation,
                        json.dumps(metadata, sort_keys=True),
                        created_at,
                    )
                )
                if len(batch) >= batch_size:
                    conn.executemany(sql, batch)
                    count += len(batch)
                    batch.clear()
            if batch:
                conn.executemany(sql, batch)
                count += len(batch)
        return count

    def list_edges_for_target(self, target_id: str, *, limit: int = 100) -> list[dict[str, Any]]:
        self.init_schema()
        with self.connect() as conn:
            rows = conn.execute(
                """
                SELECT edge_id, source_id, target_id, relation, metadata_json, created_at
                FROM provenance_edges
                WHERE target_id = ?
                ORDER BY created_at DESC
                LIMIT ?
                """,
                (target_id, max(limit, 1)),
            ).fetchall()
        return [
            {
                "edge_id": row["edge_id"],
                "source_id": row["source_id"],
                "target_id": row["target_id"],
                "relation": row["relation"],
                "metadata": json.loads(row["metadata_json"]),
                "created_at": row["created_at"],
            }
            for row in rows
        ]

    def add_replay_package(self, package: ReplayPackage) -> None:
        with self.connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO replay_packages
                (replay_package_id, artifact_id, package_json, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (
                    package.replay_package_id,
                    package.artifact_id,
                    package.model_dump_json(),
                    _dt(package.created_at),
                ),
            )

    def get_replay_package(self, artifact_id: str) -> ReplayPackage | None:
        with self.connect() as conn:
            row = conn.execute(
                """
                SELECT package_json FROM replay_packages
                WHERE artifact_id = ?
                ORDER BY created_at DESC
                LIMIT 1
                """,
                (artifact_id,),
            ).fetchone()
        return ReplayPackage.model_validate_json(row["package_json"]) if row else None

    def log(
        self,
        operation: str,
        actor: str,
        input_data: dict[str, Any],
        output_data: dict[str, Any],
        status: str,
        task_id: str | None = None,
    ) -> None:
        self.init_schema()
        with self.connect() as conn:
            conn.execute(
                """
                INSERT INTO audit_log
                (audit_id, operation, actor, task_id, input_json, output_json, status, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    f"audit_{uuid.uuid4().hex}",
                    operation,
                    actor,
                    task_id,
                    json.dumps(input_data, sort_keys=True, default=str),
                    json.dumps(output_data, sort_keys=True, default=str),
                    status,
                    datetime.now(timezone.utc).isoformat(),
                ),
            )

    def _row_to_memory(self, row: sqlite3.Row) -> MemoryObject:
        return MemoryObject(
            id=row["id"],
            type=row["type"],
            summary=row["summary"],
            content=json.loads(row["content_json"]),
            source=row["source"],
            authority=row["authority"],
            effective_start=_parse_dt(row["effective_start"]),
            effective_end=_parse_dt(row["effective_end"]),
            transaction_time=_parse_dt(row["transaction_time"]) or datetime.now(timezone.utc),
            permissions=json.loads(row["permissions_json"]),
            sensitivity=row["sensitivity"],
            version=row["version"],
            status=row["status"],
            supersedes=json.loads(row["supersedes_json"]),
            provenance_ref=row["provenance_ref"],
        )
