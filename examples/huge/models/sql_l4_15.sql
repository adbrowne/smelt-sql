---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.category,
    a.is_active,
    b.score
FROM smelt.ref('sql_l3_121') a
INNER JOIN smelt.ref('sql_l3_110') b ON a.user_id = b.user_id
