---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    COUNT(DISTINCT user_id) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.ref('py_l3_263')
GROUP BY user_id
