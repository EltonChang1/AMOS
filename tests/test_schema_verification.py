from __future__ import annotations

from amos.memory.store import MemoryStore
from amos.tools.sql_templates import payment_failure_summary_sql
from amos.verifier.schema_checks import check_schema


def test_schema_drift_old_failure_reason_is_rejected(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    _, errors = check_schema("SELECT failure_reason FROM payment_events", schema)
    assert any("failure_reason" in error for error in errors)


def test_current_schema_query_passes(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    _, errors = check_schema(payment_failure_summary_sql(), schema)
    assert errors == []


def test_schema_accepts_select_aliases_and_cte_outputs(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    sql = """
    WITH hourly AS (
      SELECT
        date_trunc('hour', event_time) AS event_hour,
        COUNT(*) AS total_attempts
      FROM payment_events
      WHERE environment = 'production'
      GROUP BY event_hour
    )
    SELECT event_hour, total_attempts
    FROM hourly
    ORDER BY total_attempts DESC
    """
    _, errors = check_schema(sql, schema)
    assert errors == []


def test_schema_rejects_unknown_column_inside_cte(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    sql = """
    WITH hourly AS (
      SELECT nonexistent_source_column AS event_hour
      FROM payment_events
    )
    SELECT event_hour FROM hourly
    """
    _, errors = check_schema(sql, schema)
    assert any("nonexistent_source_column" in error for error in errors)


def test_schema_rejects_timestamptz_against_naive_event_time(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    sql = """
    SELECT COUNT(*)
    FROM payment_events
    WHERE event_time >= TIMESTAMPTZ '2026-07-07 08:00:00+00:00'
    """

    _, errors = check_schema(sql, schema)

    assert any("session-timezone shifts" in error for error in errors)


def test_schema_accepts_timestamp_matching_event_time_type(seeded: None) -> None:
    schema = MemoryStore().get_memory("memory_schema_payment_events_v2")
    assert schema is not None
    sql = """
    SELECT COUNT(*)
    FROM payment_events
    WHERE event_time >= TIMESTAMP '2026-07-07 08:00:00'
    """

    _, errors = check_schema(sql, schema)

    assert errors == []
