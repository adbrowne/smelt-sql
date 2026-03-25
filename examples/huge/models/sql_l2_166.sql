---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.updated_at,
    a.amount,
    b.created_at
FROM smelt.ref('sql_l1_187') a
INNER JOIN smelt.ref('sql_l1_246') b ON a.user_id = b.user_id
