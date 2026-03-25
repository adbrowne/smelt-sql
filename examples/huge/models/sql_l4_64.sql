---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    COUNT(*) AS agg_0,
    AVG(price) AS agg_1,
    SUM(revenue) AS agg_2,
    MIN(created_at) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.ref('py_l3_274')
GROUP BY session_id
