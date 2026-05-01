---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    country,
    tier
FROM smelt.models.sql_l1_209
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l1_114 WHERE score >= 50
)

