---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    AVG(price) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(amount) AS agg_2,
    COUNT(*) AS agg_3
FROM smelt.ref('categories')
GROUP BY is_verified
