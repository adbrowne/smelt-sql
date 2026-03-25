---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    MIN(created_at) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(revenue) AS agg_2,
    MAX(created_at) AS agg_3,
    AVG(price) AS agg_4
FROM smelt.ref('clicks')
GROUP BY revenue
