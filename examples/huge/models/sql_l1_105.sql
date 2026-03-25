---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    SUM(quantity) AS agg_0,
    COUNT(*) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    AVG(duration_seconds) AS agg_3
FROM smelt.ref('events')
GROUP BY category
