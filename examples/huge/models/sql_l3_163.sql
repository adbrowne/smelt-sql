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
    updated_at,
    cost,
    plan_type
FROM smelt.sql_l2_221
WHERE platform = 'web'
