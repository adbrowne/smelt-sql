---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    b.discount,
    c.segment,
    c.quantity
FROM smelt.ref('events') a
INNER JOIN smelt.ref('events') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('events') c ON a.user_id = c.user_id
