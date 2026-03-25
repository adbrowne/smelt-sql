---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    SUM(amount) AS agg_0,
    SUM(revenue) AS agg_1,
    SUM(quantity) AS agg_2,
    COUNT(*) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.ref('errors')
GROUP BY amount
