---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_verified,
    a.created_at,
    b.status
FROM smelt.ref('sql_l1_220') a
INNER JOIN smelt.ref('sql_l1_186') b ON a.user_id = b.user_id
