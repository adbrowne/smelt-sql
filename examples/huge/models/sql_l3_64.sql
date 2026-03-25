---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.plan_type,
    a.ip_address,
    b.device_type
FROM smelt.ref('sql_l2_128') a
INNER JOIN smelt.ref('sql_l2_126') b ON a.user_id = b.user_id
