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
    COUNT(DISTINCT user_id) AS agg_0,
    SUM(revenue) AS agg_1,
    MIN(created_at) AS agg_2,
    AVG(duration_seconds) AS agg_3,
    COUNT(*) AS agg_4
FROM smelt.ref('sql_l1_13')
GROUP BY transaction_id
