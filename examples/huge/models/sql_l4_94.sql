---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    a.revenue,
    b.rating
FROM smelt.sql_l3_163 a
INNER JOIN smelt.sql_l3_194 b ON a.user_id = b.user_id

