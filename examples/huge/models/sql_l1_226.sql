---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    COUNT(*) AS agg_0,
    AVG(amount) AS agg_1,
    SUM(amount) AS agg_2,
    AVG(duration_seconds) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.ref('page_views')
GROUP BY event_date
