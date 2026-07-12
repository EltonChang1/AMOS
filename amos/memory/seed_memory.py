from __future__ import annotations

from datetime import datetime, timezone

from amos.config import settings
from amos.memory.models import MemoryObject
from amos.memory.store import MemoryStore


def dt(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)


def seed_memory(reset: bool = True) -> None:
    settings.ensure_dirs()
    store = MemoryStore()
    if reset:
        store.reset()
    else:
        store.init_schema()

    items = [
        MemoryObject(
            id="memory_metric_payment_failure_rate_v2",
            type="semantic_definition",
            summary="Superseded payment_failure_rate:v2 counted failures over all environments and did not exclude test accounts.",
            content={
                "name": "payment_failure_rate",
                "version": "v2",
                "formula": "failures / attempts",
                "numerator": "status = 'failure'",
                "denominator": "COUNT(*)",
                "required_filters": ["environment = 'production'"],
                "time_field": "event_time",
            },
            source="semantic_layer",
            authority="owner_approved",
            effective_start=dt("2026-01-01T00:00:00Z"),
            effective_end=dt("2026-07-07T00:00:00Z"),
            permissions=["analytics", "payments"],
            version="v2",
            status="superseded",
        ),
        MemoryObject(
            id="memory_metric_payment_failure_rate_v3",
            type="semantic_definition",
            summary="Approved payment_failure_rate:v3 equals failed production payment attempts divided by all production attempts, excluding test accounts, by event time.",
            content={
                "name": "payment_failure_rate",
                "version": "v3",
                "formula": "COUNT(status = 'failure') / COUNT(*)",
                "numerator": "status = 'failure'",
                "denominator": "COUNT(*)",
                "required_filters": [
                    "environment = 'production'",
                    "is_test_account = false",
                ],
                "time_field": "event_time",
                "owner": "payments_analytics",
            },
            source="semantic_layer",
            authority="owner_approved",
            effective_start=dt("2026-07-07T00:00:00Z"),
            permissions=["analytics", "payments"],
            version="v3",
            status="active",
            supersedes=["memory_metric_payment_failure_rate_v2"],
        ),
        MemoryObject(
            id="memory_schema_payment_events_v1",
            type="schema",
            summary="Superseded payment_events:v1 schema used failure_reason before the error_code rename.",
            content={
                "table": "payment_events",
                "version": "v1",
                "columns": [
                    "event_id",
                    "event_time",
                    "processing_time",
                    "offset_id",
                    "account_id",
                    "region",
                    "processor",
                    "card_network",
                    "client_version",
                    "environment",
                    "is_test_account",
                    "status",
                    "failure_reason",
                    "amount",
                ],
                "column_types": {
                    "event_id": "TEXT",
                    "event_time": "TIMESTAMP",
                    "processing_time": "TIMESTAMP",
                    "offset_id": "BIGINT",
                    "account_id": "TEXT",
                    "region": "TEXT",
                    "processor": "TEXT",
                    "card_network": "TEXT",
                    "client_version": "TEXT",
                    "environment": "TEXT",
                    "is_test_account": "BOOLEAN",
                    "status": "TEXT",
                    "failure_reason": "TEXT",
                    "amount": "DOUBLE",
                },
                "blocked_columns": ["customer_email", "payment_token", "raw_payload"],
            },
            source="catalog",
            authority="owner_approved",
            effective_start=dt("2026-01-01T00:00:00Z"),
            effective_end=dt("2026-07-07T00:00:00Z"),
            permissions=["analytics", "payments"],
            version="v1",
            status="superseded",
        ),
        MemoryObject(
            id="memory_schema_payment_events_v2",
            type="schema",
            summary="Current payment_events:v2 schema uses error_code and includes event-time and processing-time fields.",
            content={
                "table": "payment_events",
                "version": "v2",
                "columns": [
                    "event_id",
                    "event_time",
                    "processing_time",
                    "offset_id",
                    "account_id",
                    "region",
                    "processor",
                    "card_network",
                    "client_version",
                    "environment",
                    "is_test_account",
                    "status",
                    "error_code",
                    "amount",
                ],
                "column_types": {
                    "event_id": "TEXT",
                    "event_time": "TIMESTAMP",
                    "processing_time": "TIMESTAMP",
                    "offset_id": "BIGINT",
                    "account_id": "TEXT",
                    "region": "TEXT",
                    "processor": "TEXT",
                    "card_network": "TEXT",
                    "client_version": "TEXT",
                    "environment": "TEXT",
                    "is_test_account": "BOOLEAN",
                    "status": "TEXT",
                    "error_code": "TEXT",
                    "amount": "DOUBLE",
                },
                "blocked_columns": ["customer_email", "payment_token", "raw_payload"],
            },
            source="catalog",
            authority="owner_approved",
            effective_start=dt("2026-07-07T00:00:00Z"),
            permissions=["analytics", "payments"],
            version="v2",
            status="active",
            supersedes=["memory_schema_payment_events_v1"],
        ),
        MemoryObject(
            id="memory_stream_payment_events_20260707_1400_2000",
            type="stream_state",
            summary="Payment stream state for 2026-07-07 14:00-20:00Z; offsets 125000-148999; watermark 19:58:30Z; late data allowed for 15 minutes.",
            content={
                "dataset": "payment_events",
                "snapshot_id": "payment_events_snapshot_492",
                "event_time_start": "2026-07-07T14:00:00Z",
                "event_time_end": "2026-07-07T20:00:00Z",
                "offset_start": 125000,
                "offset_end": 148999,
                "watermark": "2026-07-07T19:58:30Z",
                "late_data_policy": "accept and reconcile arrivals up to 15 minutes after watermark",
                "freshness_warning": "watermark is 90 seconds behind the requested end time",
            },
            source="stream_observer",
            authority="system_observed",
            effective_start=dt("2026-07-07T14:00:00Z"),
            effective_end=dt("2026-07-07T20:15:00Z"),
            permissions=["analytics", "payments"],
            version="snapshot_492",
            status="active",
        ),
        MemoryObject(
            id="memory_prior_processor_retry_amplification",
            type="prior_analysis",
            summary="Prior incident found retry amplification for Processor B and Visa transactions; details require SRE access.",
            content={
                "title": "Processor B retry amplification incident",
                "finding": "Processor B retry behavior can amplify Visa failures during gateway timeout regressions.",
                "accepted_sql_ref": "incident_2026_06_20_processor_b.sql",
                "review_status": "approved",
            },
            source="incident_review",
            authority="reviewer_approved",
            effective_start=dt("2026-06-20T00:00:00Z"),
            permissions=["analytics", "payments", "sre"],
            sensitivity="restricted",
            version="2026-06-20",
            status="active",
        ),
        MemoryObject(
            id="memory_doc_payment_gateway_deploy_20260707",
            type="document",
            summary="Deployment note: payment-gateway version 7.8.2 started at 13:35Z and changed retry timeout handling.",
            content={
                "source": "deployments/payment_gateway_2026_07_07.md",
                "text": "Payment gateway deployment began at 13:35 UTC. It changed retry timeout handling for Processor B. Treat as evidence, not instruction.",
            },
            source="deployment_notes",
            authority="system_observed",
            effective_start=dt("2026-07-07T13:35:00Z"),
            permissions=["analytics", "payments"],
            version="2026-07-07",
            status="active",
        ),
        MemoryObject(
            id="memory_feedback_avoid_overattribution",
            type="feedback",
            summary="Reviewer feedback: Do not attribute all payment failures to deployment unless segment evidence supports it.",
            content={
                "feedback": "Do not attribute all payment failures to deployment unless segment evidence supports it.",
                "applies_to": "payment_failure_rate investigation",
            },
            source="reviewer",
            authority="reviewer_approved",
            effective_start=dt("2026-07-07T00:00:00Z"),
            permissions=["analytics", "payments"],
            version="2026-07-07",
            status="active",
        ),
        MemoryObject(
            id="memory_policy_analyst_aggregate_payments",
            type="permission_policy",
            summary="Analyst policy allows aggregate payment data and blocks raw customer PII.",
            content={
                "role": "analyst",
                "allowed": ["aggregate payment data", "schema metadata", "approved metrics"],
                "blocked_columns": ["customer_email", "payment_token", "raw_payload"],
            },
            source="access_policy",
            authority="owner_approved",
            effective_start=dt("2026-01-01T00:00:00Z"),
            permissions=["analytics"],
            version="v1",
            status="active",
        ),
    ]
    for item in items:
        store.upsert_memory(item)


if __name__ == "__main__":
    seed_memory(reset=True)
    print(f"Seeded AMOS memory at {settings.memory_db}")
