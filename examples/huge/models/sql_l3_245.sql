---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    tier,
    is_verified
FROM smelt.sql_l2_33
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_9 WHERE category IS NOT NULL
)

