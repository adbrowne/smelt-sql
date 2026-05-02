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
    a.score,
    b.order_id
FROM smelt.models.sql_l3_133 a
INNER JOIN smelt.models.sql_l3_234 b ON a.user_id = b.user_id

