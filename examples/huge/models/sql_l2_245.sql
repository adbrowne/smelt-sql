---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    MAX(created_at) AS agg_0,
    SUM(quantity) AS agg_1,
    SUM(amount) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.ref('py_l1_470')
GROUP BY created_at
