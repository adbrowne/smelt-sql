---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    b.device_type,
    c.tier,
    c.os_name
FROM smelt.ref('sql_l1_50') a
INNER JOIN smelt.ref('sql_l1_116') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_71') c ON a.user_id = c.user_id
