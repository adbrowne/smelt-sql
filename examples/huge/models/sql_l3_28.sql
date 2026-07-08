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
    updated_at,
    price,
    is_active,
    referrer
FROM smelt.sql_l2_59
WHERE score >= 50
