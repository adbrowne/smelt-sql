---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.device_type,
    b.event_time,
    c.referrer,
    c.price
FROM smelt.ref('sql_l1_189') a
INNER JOIN smelt.ref('sql_l1_189') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_189') c ON a.user_id = c.user_id
