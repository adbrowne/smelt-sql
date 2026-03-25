---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.plan_type,
    a.ip_address,
    b.device_type
FROM smelt.ref('sql_l2_44') a
INNER JOIN smelt.ref('sql_l2_85') b ON a.user_id = b.user_id
