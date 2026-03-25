---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.region,
    b.segment,
    c.tier,
    c.channel
FROM smelt.ref('py_l3_338') a
INNER JOIN smelt.ref('py_l3_338') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l3_338') c ON a.user_id = c.user_id
