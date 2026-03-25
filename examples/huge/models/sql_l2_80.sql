---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1,
    SUM(quantity) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3
FROM smelt.ref('py_l1_485')
GROUP BY country
