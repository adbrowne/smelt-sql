---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    COUNT(DISTINCT user_id) AS agg_0,
    COUNT(*) AS agg_1,
    AVG(duration_seconds) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.ref('py_l1_396')
GROUP BY is_verified
