---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    a.duration_seconds,
    b.cost
FROM smelt.ref('sql_l2_75') a
LEFT JOIN smelt.ref('sql_l2_71') b ON a.user_id = b.user_id
