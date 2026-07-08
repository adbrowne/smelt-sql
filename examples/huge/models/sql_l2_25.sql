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
    device_type,
    cohort_date,
    created_at
FROM smelt.sql_l1_120
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_120 WHERE status = 'active'
)
