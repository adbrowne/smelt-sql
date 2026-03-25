---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    AVG(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(quantity) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.ref('sql_l1_170')
GROUP BY plan_type
