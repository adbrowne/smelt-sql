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
    a.revenue,
    a.order_id,
    b.event_time
FROM smelt.sql_l3_138 a
LEFT JOIN smelt.sql_l3_150 b ON a.user_id = b.user_id
