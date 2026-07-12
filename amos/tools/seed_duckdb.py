from __future__ import annotations

from contextlib import contextmanager
import random
from datetime import datetime, timedelta, timezone
from pathlib import Path
import tempfile
import uuid

try:
    import fcntl
except ImportError:  # pragma: no cover - fcntl is unavailable on Windows.
    fcntl = None  # type: ignore[assignment]

import duckdb

from amos.config import settings


START = datetime(2026, 7, 7, 8, 0, tzinfo=timezone.utc)
END = datetime(2026, 7, 7, 20, 0, tzinfo=timezone.utc)
SPIKE_START = datetime(2026, 7, 7, 14, 0, tzinfo=timezone.utc)
DEPLOYMENT_TIME = datetime(2026, 7, 7, 13, 35, tzinfo=timezone.utc)


def seed_duckdb() -> None:
    settings.ensure_dirs()
    with _duckdb_seed_lock():
        _seed_duckdb_unlocked()


def _seed_duckdb_unlocked() -> None:
    if settings.rotate_analytics_db_on_seed:
        settings.use_paths(analytics_db=_rotated_db_path(settings.analytics_db))
    elif settings.analytics_db.exists():
        settings.analytics_db.unlink()

    random.seed(42)
    accounts = _accounts()
    events = _payment_events(accounts)

    with duckdb.connect(str(settings.analytics_db)) as conn:
        conn.execute(
            """
            CREATE TABLE payment_events (
                event_id TEXT,
                event_time TIMESTAMP,
                processing_time TIMESTAMP,
                offset_id BIGINT,
                account_id TEXT,
                region TEXT,
                processor TEXT,
                card_network TEXT,
                client_version TEXT,
                environment TEXT,
                is_test_account BOOLEAN,
                status TEXT,
                error_code TEXT,
                amount DOUBLE
            )
            """
        )
        conn.execute(
            """
            CREATE TABLE account_dim (
                account_id TEXT,
                segment TEXT,
                is_internal BOOLEAN,
                created_at TIMESTAMP
            )
            """
        )
        conn.execute(
            """
            CREATE TABLE deployment_events (
                deployment_id TEXT,
                service TEXT,
                deployed_at TIMESTAMP,
                summary TEXT,
                owner TEXT
            )
            """
        )
        conn.executemany(
            "INSERT INTO account_dim VALUES (?, ?, ?, ?)",
            accounts,
        )
        conn.executemany(
            """
            INSERT INTO payment_events VALUES
            (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            events,
        )
        conn.execute(
            "INSERT INTO deployment_events VALUES (?, ?, ?, ?, ?)",
            (
                "deploy_payment_gateway_20260707_1335",
                "payment-gateway",
                DEPLOYMENT_TIME,
                "Version 7.8.2 changed retry timeout handling for Processor B.",
                "payments-platform",
            ),
        )


@contextmanager
def _duckdb_seed_lock():
    if fcntl is None:
        yield
        return

    lock_path = Path(tempfile.gettempdir()) / "amos_duckdb_seed.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("w", encoding="utf-8") as lock_file:
        fcntl.flock(lock_file, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock_file, fcntl.LOCK_UN)


def _rotated_db_path(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    stem = path.stem.split(".seed_")[0]
    return path.with_name(f"{stem}.seed_{uuid.uuid4().hex[:12]}{path.suffix}")


def _accounts() -> list[tuple[object, ...]]:
    segments = ["enterprise", "startup", "consumer", "marketplace"]
    rows = []
    for index in range(1, 301):
        rows.append(
            (
                f"acct_{index:04d}",
                segments[index % len(segments)],
                index % 47 == 0,
                START - timedelta(days=index % 180),
            )
        )
    return rows


def _payment_events(accounts: list[tuple[object, ...]]) -> list[tuple[object, ...]]:
    processors = ["Processor A", "Processor B", "Processor C"]
    networks = ["Visa", "Mastercard", "Amex"]
    regions = ["NA", "EU", "APAC"]
    client_versions = ["ios-6.2", "android-5.9", "web-12.4"]
    rows: list[tuple[object, ...]] = []
    offset = 100000
    cursor = START
    while cursor < END:
        for i in range(20):
            account_id = accounts[(offset + i) % len(accounts)][0]
            processor = random.choices(processors, weights=[0.42, 0.34, 0.24])[0]
            network = random.choices(networks, weights=[0.52, 0.36, 0.12])[0]
            environment = "test" if random.random() < 0.025 else "production"
            is_test = environment == "test" or str(account_id).endswith("047")
            failure_rate = 0.02
            if cursor >= SPIKE_START:
                failure_rate = 0.038
                if processor == "Processor B" and network == "Visa":
                    failure_rate = 0.16
                elif processor == "Processor B":
                    failure_rate = 0.08
                elif network == "Visa":
                    failure_rate = 0.055
            status = "failure" if random.random() < failure_rate else "success"
            error_code = None
            if status == "failure":
                error_code = random.choices(
                    ["processor_timeout", "issuer_declined", "network_unavailable", "fraud_reject"],
                    weights=[0.58 if cursor >= SPIKE_START else 0.18, 0.24, 0.12, 0.06],
                )[0]
            processing_lag = random.randint(4, 70)
            if cursor >= SPIKE_START and random.random() < 0.015:
                processing_lag += random.randint(600, 1200)
            rows.append(
                (
                    f"evt_{offset}",
                    cursor,
                    cursor + timedelta(seconds=processing_lag),
                    offset,
                    account_id,
                    random.choice(regions),
                    processor,
                    network,
                    random.choice(client_versions),
                    environment,
                    bool(is_test),
                    status,
                    error_code,
                    round(random.uniform(7.0, 450.0), 2),
                )
            )
            offset += 1
        cursor += timedelta(minutes=1)
    return rows


if __name__ == "__main__":
    seed_duckdb()
    print(f"Seeded DuckDB analytics data at {settings.analytics_db}")
