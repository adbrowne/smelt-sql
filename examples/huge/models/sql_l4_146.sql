---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.order_id,
    b.event_date,
    c.segment,
    c.status
FROM smelt.ref('sql_l3_187') a
INNER JOIN smelt.ref('sql_l3_72') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_187') c ON a.user_id = c.user_id
