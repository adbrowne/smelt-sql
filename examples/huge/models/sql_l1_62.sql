---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    AVG(duration_seconds) AS agg_0,
    AVG(amount) AS agg_1,
    MIN(created_at) AS agg_2,
    COUNT(*) AS agg_3
FROM smelt.ref('users')
GROUP BY category
