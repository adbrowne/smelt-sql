---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    MAX(created_at) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2,
    COUNT(*) AS agg_3,
    MIN(created_at) AS agg_4
FROM smelt.ref('py_l1_272')
GROUP BY browser
