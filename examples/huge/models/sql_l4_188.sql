---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cost,
    b.price,
    c.cohort_date,
    c.tier
FROM smelt.ref('sql_l3_158') a
INNER JOIN smelt.ref('sql_l3_158') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_158') c ON a.user_id = c.user_id
