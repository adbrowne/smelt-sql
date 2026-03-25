---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.ref('sql_l2_16')
GROUP BY revenue
