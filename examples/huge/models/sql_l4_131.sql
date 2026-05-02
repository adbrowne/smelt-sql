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
    channel
FROM smelt.models.sql_l3_25
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l3_189 WHERE category IS NOT NULL
)

