---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    profit,
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.ref('sql_l1_43')
GROUP BY profit
