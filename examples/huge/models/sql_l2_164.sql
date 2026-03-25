---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    SUM(revenue) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.ref('sql_l1_91')
GROUP BY plan_type
