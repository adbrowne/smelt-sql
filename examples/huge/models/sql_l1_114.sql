---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    AVG(duration_seconds) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    MIN(created_at) AS agg_2,
    MAX(created_at) AS agg_3,
    SUM(quantity) AS agg_4
FROM smelt.models.signups
GROUP BY rating

