---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.platform,
    b.device_type,
    c.tier,
    c.os_name
FROM smelt.ref('sql_l1_55') a
INNER JOIN smelt.ref('sql_l1_67') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_55') c ON a.user_id = c.user_id
