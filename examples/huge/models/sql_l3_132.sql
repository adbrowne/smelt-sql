---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    SUM(quantity) AS agg_3,
    AVG(amount) AS agg_4
FROM smelt.ref('py_l2_346')
GROUP BY is_active
