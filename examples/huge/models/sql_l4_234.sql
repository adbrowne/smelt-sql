---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    AVG(price) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3,
    AVG(duration_seconds) AS agg_4
FROM smelt.ref('sql_l3_193')
GROUP BY order_id
