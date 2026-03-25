---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.status,
    a.rating,
    b.updated_at
FROM smelt.ref('sql_l2_56') a
LEFT JOIN smelt.ref('sql_l2_37') b ON a.user_id = b.user_id
