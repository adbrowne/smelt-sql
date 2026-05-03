---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    duration_seconds,
    amount
FROM smelt.sql_l1_232
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_63 WHERE score >= 50
)

