---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.device_type,
    b.event_time,
    c.referrer,
    c.price
FROM smelt.ref('sql_l1_28') a
INNER JOIN smelt.ref('sql_l1_28') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_28') c ON a.user_id = c.user_id
