---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    MAX(created_at) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.ref('py_l3_404')
GROUP BY plan_type
