from __future__ import annotations


PAYMENT_WINDOW_START = "2026-07-07T14:00:00+00:00"
PAYMENT_WINDOW_END = "2026-07-07T20:00:00+00:00"
PAYMENT_PREVIOUS_START = "2026-07-07T08:00:00+00:00"
PAYMENT_PREVIOUS_END = "2026-07-07T14:00:00+00:00"


def payment_failure_summary_sql() -> str:
    return f"""
    WITH base AS (
      SELECT
        CASE
          WHEN event_time >= TIMESTAMP '{PAYMENT_WINDOW_START}' AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}' THEN 'current'
          WHEN event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}' AND event_time < TIMESTAMP '{PAYMENT_PREVIOUS_END}' THEN 'previous'
        END AS period,
        status
      FROM payment_events
      WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
        AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
        AND environment = 'production'
        AND is_test_account = false
    )
    SELECT
      period,
      COUNT(*) AS attempts,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failures,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END)::DOUBLE / COUNT(*) AS failure_rate
    FROM base
    WHERE period IS NOT NULL
    GROUP BY period
    ORDER BY period
    """


def payment_failure_concentration_sql() -> str:
    return f"""
    SELECT
      processor,
      card_network,
      COUNT(*) AS attempts,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failures,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END)::DOUBLE / COUNT(*) AS failure_rate
    FROM payment_events
    WHERE event_time >= TIMESTAMP '{PAYMENT_WINDOW_START}'
      AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
      AND environment = 'production'
      AND is_test_account = false
    GROUP BY processor, card_network
    HAVING COUNT(*) >= 25
    ORDER BY failure_rate DESC, failures DESC
    LIMIT 5
    """


def payment_failure_timeseries_sql() -> str:
    return f"""
    SELECT
      date_trunc('hour', event_time) AS bucket,
      COUNT(*) AS attempts,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END) AS failures,
      SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END)::DOUBLE / COUNT(*) AS failure_rate
    FROM payment_events
    WHERE event_time >= TIMESTAMP '{PAYMENT_PREVIOUS_START}'
      AND event_time < TIMESTAMP '{PAYMENT_WINDOW_END}'
      AND environment = 'production'
      AND is_test_account = false
    GROUP BY bucket
    ORDER BY bucket
    """
