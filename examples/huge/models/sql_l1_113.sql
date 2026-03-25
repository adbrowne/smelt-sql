---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    COUNT(DISTINCT user_id) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2
FROM smelt.ref('logs')
GROUP BY category
