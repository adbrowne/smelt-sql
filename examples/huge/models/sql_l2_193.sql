---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    a.rating,
    b.ip_address
FROM smelt.sql_l1_200 a
INNER JOIN smelt.sql_l1_200 b ON a.user_id = b.user_id

