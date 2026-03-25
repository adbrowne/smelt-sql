---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    a.updated_at,
    b.os_name
FROM smelt.ref('sql_l1_80') a
LEFT JOIN smelt.ref('sql_l1_80') b ON a.user_id = b.user_id
