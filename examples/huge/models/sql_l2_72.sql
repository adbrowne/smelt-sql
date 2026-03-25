---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    AVG(price) AS agg_0,
    SUM(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(revenue) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.ref('sql_l1_149')
GROUP BY transaction_id
