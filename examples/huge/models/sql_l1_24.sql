---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_type,
    b.referrer,
    c.os_name,
    c.cost
FROM smelt.ref('payments') a
INNER JOIN smelt.ref('payments') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('payments') c ON a.user_id = c.user_id
