---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.category,
    b.tier,
    c.duration_seconds,
    c.email_domain
FROM smelt.ref('sql_l3_152') a
INNER JOIN smelt.ref('sql_l3_152') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_152') c ON a.user_id = c.user_id
