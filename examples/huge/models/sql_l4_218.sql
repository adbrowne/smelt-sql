---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.profit,
    a.plan_type,
    b.channel
FROM smelt.ref('sql_l3_38') a
LEFT JOIN smelt.ref('py_l3_400') b ON a.user_id = b.user_id
