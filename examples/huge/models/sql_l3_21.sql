---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.ref('sql_l2_95')
GROUP BY amount
