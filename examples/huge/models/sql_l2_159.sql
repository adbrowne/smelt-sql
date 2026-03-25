---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.ref('py_l1_333')
GROUP BY is_verified
