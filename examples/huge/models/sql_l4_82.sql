---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.revenue,
    a.order_id,
    b.event_time
FROM smelt.ref('sql_l3_58') a
LEFT JOIN smelt.ref('sql_l3_214') b ON a.user_id = b.user_id
