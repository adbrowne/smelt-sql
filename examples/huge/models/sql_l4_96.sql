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
    segment,
    is_verified,
    referrer
FROM smelt.sql_l3_105
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_105 WHERE score >= 50
)

