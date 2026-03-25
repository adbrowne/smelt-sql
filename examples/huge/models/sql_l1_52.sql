---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    COUNT(DISTINCT user_id) AS agg_0,
    MAX(created_at) AS agg_1,
    SUM(revenue) AS agg_2
FROM smelt.ref('reviews')
GROUP BY category
