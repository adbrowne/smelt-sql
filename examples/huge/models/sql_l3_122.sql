---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.revenue,
    b.channel,
    c.segment,
    c.transaction_id
FROM smelt.models.sql_l2_131 a
INNER JOIN smelt.models.sql_l2_215 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l2_236 c ON a.user_id = c.user_id

