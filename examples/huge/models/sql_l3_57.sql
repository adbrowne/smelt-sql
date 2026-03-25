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
    b.transaction_id,
    c.event_type,
    c.event_date
FROM smelt.ref('sql_l2_69') a
INNER JOIN smelt.ref('sql_l2_212') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_69') c ON a.user_id = c.user_id
