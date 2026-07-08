---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    country,
    campaign_id
FROM smelt.sql_l2_74
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_65 WHERE status = 'active'
)
