---
materialization: table
refresh: incremental
---
-- SuccessionOrderNotMonotoneClock: the window orders by `customer_id`,
-- which does not trace to the source's declared event_time_column
-- (`changed_at`) (`docs/specs/model_properties.md` §"Event-time
-- monotonicity trace").
SELECT
    customer_id,
    changed_at,
    LEAD(customer_id) OVER (PARTITION BY customer_id ORDER BY customer_id) AS next_customer_id
FROM smelt.sources.succession_changes
