---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    SUM(amount) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.ref('sql_l3_206')
GROUP BY platform
