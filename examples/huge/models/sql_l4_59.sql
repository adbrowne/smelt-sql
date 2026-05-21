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
    a.tier,
    a.ip_address,
    b.amount
FROM smelt.sql_l3_207 a
LEFT JOIN smelt.sql_l3_129 b ON a.user_id = b.user_id

