---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    COUNT(DISTINCT user_id) AS agg_0,
    SUM(revenue) AS agg_1,
    MIN(created_at) AS agg_2,
    AVG(duration_seconds) AS agg_3,
    COUNT(*) AS agg_4
FROM smelt.ref('py_l1_361')
GROUP BY transaction_id
