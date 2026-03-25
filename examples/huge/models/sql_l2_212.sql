---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    AVG(amount) AS agg_0,
    SUM(quantity) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(amount) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.ref('sql_l1_160')
GROUP BY category
