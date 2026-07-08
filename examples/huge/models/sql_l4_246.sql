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
    a.order_id,
    a.cohort_date,
    b.event_time
FROM smelt.sql_l3_63 a
LEFT JOIN smelt.sql_l3_238 b ON a.user_id = b.user_id
