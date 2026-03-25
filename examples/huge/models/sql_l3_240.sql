---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    b.page_path,
    c.device_type,
    c.cost
FROM smelt.ref('sql_l2_249') a
INNER JOIN smelt.ref('sql_l2_193') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_249') c ON a.user_id = c.user_id
