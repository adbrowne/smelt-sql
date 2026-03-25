---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    AVG(price) AS agg_0,
    SUM(revenue) AS agg_1,
    MAX(created_at) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3,
    COUNT(*) AS agg_4
FROM smelt.ref('py_l2_321')
GROUP BY rating
