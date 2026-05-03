---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    user_id,
    tier
FROM smelt.sql_l3_210
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_59 WHERE score >= 50
)

