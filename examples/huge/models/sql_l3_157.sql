---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    cohort_date,
    created_at
FROM smelt.sql_l2_103
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_135 WHERE quantity > 0
)
