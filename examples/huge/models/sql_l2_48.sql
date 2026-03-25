---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.os_name,
    a.updated_at,
    b.quantity
FROM smelt.ref('sql_l1_80') a
INNER JOIN smelt.ref('sql_l1_80') b ON a.user_id = b.user_id
