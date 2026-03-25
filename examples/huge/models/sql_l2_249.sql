---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    SUM(quantity) AS agg_0,
    SUM(revenue) AS agg_1,
    AVG(amount) AS agg_2
FROM smelt.ref('sql_l1_15')
GROUP BY created_at
