---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    category,
    session_id
FROM smelt.ref('transactions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('transactions') WHERE score >= 50
)
