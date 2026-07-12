from __future__ import annotations

from amos.memory.store import MemoryStore
from amos.tools.sql_templates import PAYMENT_PREVIOUS_START, PAYMENT_WINDOW_END, payment_failure_summary_sql
from amos.verifier.metric_checks import check_metric_rules


def test_metric_query_requires_test_account_filter(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    bad_sql = payment_failure_summary_sql().replace("AND is_test_account = false", "")
    _, errors = check_metric_rules(bad_sql, metric)
    assert any("is_test_account = false" in error for error in errors)


def test_metric_query_passes_required_rules(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    _, errors = check_metric_rules(payment_failure_summary_sql(), metric)
    assert errors == []


def test_metric_query_accepts_ast_equivalent_filters(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    sql = f"""
    SELECT
      COUNT_IF(p.status = 'failure')::DOUBLE / COUNT(*) AS failure_rate
    FROM payment_events AS p
    WHERE p.event_time BETWEEN TIMESTAMP '{PAYMENT_PREVIOUS_START}' AND TIMESTAMP '{PAYMENT_WINDOW_END}'
      AND 'production' = p.environment
      AND NOT p.is_test_account
    """

    _, errors = check_metric_rules(sql, metric)

    assert errors == []


def test_metric_query_rejects_comment_or_string_only_matches(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    sql = f"""
    SELECT
      'status = failure count(*) environment = production is_test_account = false' AS fake_metric
      -- status = 'failure' COUNT(*) environment = 'production' is_test_account = false
    FROM payment_events
    WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
      AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
    """

    _, errors = check_metric_rules(sql, metric)

    assert any("status = 'failure'" in error for error in errors)
    assert any("COUNT(*)" in error for error in errors)
    assert any("environment = 'production'" in error for error in errors)
    assert any("is_test_account = false" in error for error in errors)


def test_metric_accepts_time_conditioned_total_attempt_denominator(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    sql = f"""
    SELECT
      COUNT(*) FILTER (
        WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
          AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
          AND status = 'failure'
      )::DOUBLE
      / NULLIF(
          COUNT(*) FILTER (
            WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
              AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
          ),
          0
        ) AS failure_rate
    FROM payment_events
    WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
      AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
      AND environment = 'production'
      AND is_test_account = false
    """
    _, errors = check_metric_rules(sql, metric)
    assert errors == []


def test_metric_rejects_outcome_filtered_count_as_only_denominator(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    sql = f"""
    SELECT
      COUNT(*) FILTER (WHERE status = 'failure')::DOUBLE
      / NULLIF(COUNT(*) FILTER (WHERE status = 'success'), 0) AS failure_rate
    FROM payment_events
    WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
      AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
      AND environment = 'production'
      AND is_test_account = false
    """
    _, errors = check_metric_rules(sql, metric)
    assert any("COUNT(*)" in error for error in errors)


def test_metric_accepts_required_filters_in_join_condition(seeded: None) -> None:
    metric = MemoryStore().get_memory("memory_metric_payment_failure_rate_v3")
    assert metric is not None
    sql = f"""
    WITH periods(period, start_time, end_time) AS (
      VALUES
        ('previous', TIMESTAMP '{PAYMENT_PREVIOUS_START}', TIMESTAMP '2026-07-07T14:00:00+00:00'),
        ('current', TIMESTAMP '2026-07-07T14:00:00+00:00', TIMESTAMP '{PAYMENT_WINDOW_END}')
    )
    SELECT
      p.period,
      COUNT(*) AS attempts,
      COUNT(*) FILTER (WHERE e.status = 'failure') AS failures,
      COUNT(*) FILTER (WHERE e.status = 'failure')::DOUBLE / NULLIF(COUNT(*), 0) AS failure_rate
    FROM periods AS p
    LEFT JOIN payment_events AS e
      ON e.event_time >= p.start_time
      AND e.event_time < p.end_time
      AND e.environment = 'production'
      AND e.is_test_account = false
    GROUP BY p.period
    """

    _, errors = check_metric_rules(sql, metric)

    assert errors == []
