---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    SUM(revenue) AS agg_0,
    COUNT(*) AS agg_1,
    SUM(quantity) AS agg_2
FROM smelt.ref('categories')
GROUP BY cost
