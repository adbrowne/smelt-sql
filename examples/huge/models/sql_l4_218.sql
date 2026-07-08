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
    a.profit,
    a.plan_type,
    b.channel
FROM smelt.sql_l3_124 a
LEFT JOIN smelt.sql_l3_124 b ON a.user_id = b.user_id
