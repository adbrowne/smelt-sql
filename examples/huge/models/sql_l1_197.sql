---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    a.is_active,
    b.user_id
FROM smelt.ref('sessions') a
INNER JOIN smelt.ref('sessions') b ON a.user_id = b.user_id
