---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.quantity,
    b.campaign_id,
    c.discount,
    c.profit
FROM smelt.ref('py_l2_488') a
INNER JOIN smelt.ref('py_l2_291') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_488') c ON a.user_id = c.user_id
