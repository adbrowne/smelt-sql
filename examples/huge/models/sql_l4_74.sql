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
    campaign_id,
    event_time,
    category
FROM smelt.sql_l3_193
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_193 WHERE platform = 'web'
)

