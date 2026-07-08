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
    os_name,
    category,
    revenue
FROM smelt.sql_l3_207
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_49 WHERE score >= 50
)
