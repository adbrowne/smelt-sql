---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.segment,
    b.updated_at,
    c.referrer,
    c.price
FROM smelt.ref('sql_l1_184') a
INNER JOIN smelt.ref('sql_l1_184') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_184') c ON a.user_id = c.user_id
