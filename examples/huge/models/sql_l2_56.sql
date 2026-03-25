---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    MIN(created_at) AS agg_0,
    SUM(revenue) AS agg_1,
    COUNT(*) AS agg_2,
    AVG(amount) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.ref('py_l1_331')
GROUP BY browser
