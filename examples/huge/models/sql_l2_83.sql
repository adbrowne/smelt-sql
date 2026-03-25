---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    b.os_name,
    c.tier,
    c.region
FROM smelt.ref('sql_l1_15') a
INNER JOIN smelt.ref('sql_l1_184') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_15') c ON a.user_id = c.user_id
