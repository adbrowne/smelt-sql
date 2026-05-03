---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    AVG(price) AS agg_0,
    SUM(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    SUM(revenue) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.sql_l1_101
GROUP BY transaction_id

