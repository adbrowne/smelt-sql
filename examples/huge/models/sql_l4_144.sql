---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.cost,
    b.country,
    c.created_at,
    c.event_date
FROM smelt.ref('sql_l3_97') a
INNER JOIN smelt.ref('sql_l3_97') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_97') c ON a.user_id = c.user_id
