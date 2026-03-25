---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    a.updated_at,
    b.quantity
FROM smelt.ref('sql_l1_53') a
INNER JOIN smelt.ref('sql_l1_214') b ON a.user_id = b.user_id
