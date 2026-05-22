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
    a.ip_address,
    b.cost,
    c.updated_at,
    c.event_date
FROM smelt.sql_l3_143 a
INNER JOIN smelt.sql_l3_1 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_104 c ON a.user_id = c.user_id
