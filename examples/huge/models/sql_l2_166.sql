---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.updated_at,
    a.amount,
    b.created_at
FROM smelt.sql_l1_212 a
INNER JOIN smelt.sql_l1_83 b ON a.user_id = b.user_id

