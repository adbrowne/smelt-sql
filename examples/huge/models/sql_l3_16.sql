---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    quantity,
    SUM(quantity) AS agg_0,
    AVG(duration_seconds) AS agg_1
FROM smelt.ref('sql_l2_89')
GROUP BY quantity
