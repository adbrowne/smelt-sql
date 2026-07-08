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
    campaign_id,
    product_id,
    tier
FROM smelt.sql_l2_15
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_235 WHERE platform = 'web'
)
