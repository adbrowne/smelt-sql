---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    MIN(created_at) AS agg_0,
    SUM(revenue) AS agg_1,
    COUNT(*) AS agg_2,
    AVG(amount) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.ref('sql_l1_140')
GROUP BY browser
