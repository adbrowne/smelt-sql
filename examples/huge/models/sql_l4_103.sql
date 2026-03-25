---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.plan_type,
    a.event_type,
    b.region
FROM smelt.ref('sql_l3_65') a
LEFT JOIN smelt.ref('py_l3_453') b ON a.user_id = b.user_id
