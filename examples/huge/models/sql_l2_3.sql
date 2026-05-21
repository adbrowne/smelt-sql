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
    a.transaction_id,
    a.event_type,
    b.os_name
FROM smelt.sql_l1_203 a
LEFT JOIN smelt.sql_l1_214 b ON a.user_id = b.user_id

