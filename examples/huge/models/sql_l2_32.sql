---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    region,
    price
FROM smelt.models.sql_l1_98
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_80 WHERE platform = 'web'
)

