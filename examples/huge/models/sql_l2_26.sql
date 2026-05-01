---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    MAX(created_at) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2,
    COUNT(*) AS agg_3,
    MIN(created_at) AS agg_4
FROM smelt.models.sql_l1_205
GROUP BY browser

