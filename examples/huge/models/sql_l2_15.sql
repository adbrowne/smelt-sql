---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    AVG(duration_seconds) AS agg_0,
    SUM(amount) AS agg_1,
    COUNT(*) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3
FROM smelt.ref('sql_l1_49')
GROUP BY price
