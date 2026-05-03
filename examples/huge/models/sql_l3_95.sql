---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.created_at,
    b.revenue,
    c.rating,
    c.duration_seconds
FROM smelt.sql_l2_156 a
INNER JOIN smelt.sql_l2_171 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_16 c ON a.user_id = c.user_id

