CREATE TABLE subscription_events (
    event_date TEXT NOT NULL,
    account_id TEXT NOT NULL,
    segment TEXT NOT NULL,
    plan_tier TEXT NOT NULL,
    environment TEXT NOT NULL,
    is_test_account INTEGER NOT NULL CHECK (is_test_account IN (0, 1)),
    churned INTEGER NOT NULL CHECK (churned IN (0, 1)),
    churn_type TEXT,
    churn_reason TEXT,
    monthly_recurring_revenue INTEGER NOT NULL,
    support_contact_count INTEGER NOT NULL,
    customer_email TEXT NOT NULL,
    raw_support_note TEXT
);

WITH RECURSIVE
    accounts(account_number) AS (
        VALUES (1)
        UNION ALL
        SELECT account_number + 1 FROM accounts WHERE account_number < 2000
    ),
    periods(period, period_start, churn_cutoff) AS (
        VALUES
            ('baseline', '2026-07-13', 62),
            ('current', '2026-07-20', 108)
    )
INSERT INTO subscription_events
SELECT
    date(period_start, printf('+%d days', (account_number - 1) % 7)),
    printf('acct_%s_%04d', period, account_number),
    'SMB',
    CASE
        WHEN account_number % 5 < 3 THEN 'Starter'
        WHEN account_number % 5 = 3 THEN 'Growth'
        ELSE 'Scale'
    END,
    'production',
    0,
    CASE WHEN account_number <= churn_cutoff THEN 1 ELSE 0 END,
    CASE
        WHEN account_number > churn_cutoff THEN NULL
        WHEN period = 'current' AND account_number <= 60 THEN 'involuntary'
        WHEN account_number % 2 = 0 THEN 'voluntary'
        ELSE 'involuntary'
    END,
    CASE
        WHEN account_number > churn_cutoff THEN NULL
        WHEN period = 'current' AND account_number <= 60 THEN 'payment_failure'
        WHEN account_number % 2 = 0 THEN 'product_fit'
        ELSE 'billing'
    END,
    49 + (account_number % 7) * 25,
    account_number % 4,
    printf('customer-%04d@example.invalid', account_number),
    CASE
        WHEN period = 'current' AND account_number = 1
        THEN 'WAREHOUSE_RAW_CANARY_9f1c2e7b'
        ELSE 'restricted support detail'
    END
FROM accounts CROSS JOIN periods;

CREATE INDEX idx_subscription_events_metric
    ON subscription_events(event_date, segment, environment, is_test_account);
