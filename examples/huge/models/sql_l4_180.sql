---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_type,
    b.product_id,
    c.os_name,
    c.channel
FROM smelt.ref('py_l3_352') a
INNER JOIN smelt.ref('py_l3_352') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_352') c ON a.user_id = c.user_id
