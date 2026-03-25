---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    b.channel,
    c.revenue,
    c.device_type
FROM smelt.ref('sql_l3_110') a
INNER JOIN smelt.ref('sql_l3_110') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_110') c ON a.user_id = c.user_id
