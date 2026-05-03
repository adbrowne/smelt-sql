---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    revenue,
    platform
FROM smelt.transactions
WHERE user_id IN (
    SELECT user_id FROM smelt.transactions WHERE status = 'active'
)

