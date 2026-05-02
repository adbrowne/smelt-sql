---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    SUM(amount) AS agg_0,
    AVG(price) AS agg_1,
    COUNT(*) AS agg_2
FROM smelt.models.page_views
GROUP BY channel

