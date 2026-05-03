---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    AVG(duration_seconds) AS agg_0,
    SUM(revenue) AS agg_1,
    AVG(price) AS agg_2,
    MAX(created_at) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.payments
GROUP BY referrer

