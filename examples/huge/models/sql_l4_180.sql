---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_type,
    b.product_id,
    c.os_name,
    c.channel
FROM smelt.ref('sql_l3_55') a
INNER JOIN smelt.ref('sql_l3_20') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_53') c ON a.user_id = c.user_id
