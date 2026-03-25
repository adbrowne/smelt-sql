---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_time,
    a.rating,
    b.ip_address
FROM smelt.ref('py_l1_468') a
INNER JOIN smelt.ref('py_l1_452') b ON a.user_id = b.user_id
