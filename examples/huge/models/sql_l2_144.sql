---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.transaction_id,
    a.os_name,
    b.referrer
FROM smelt.ref('py_l1_460') a
LEFT JOIN smelt.ref('py_l1_353') b ON a.user_id = b.user_id
