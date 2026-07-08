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
    cohort_date,
    device_type,
    email_domain
FROM smelt.sql_l1_121
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_132 WHERE amount > 0
)
