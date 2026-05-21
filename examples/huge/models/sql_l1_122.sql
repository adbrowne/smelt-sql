---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    category,
    session_id
FROM smelt.transactions
WHERE user_id IN (
    SELECT user_id FROM smelt.transactions WHERE score >= 50
)

