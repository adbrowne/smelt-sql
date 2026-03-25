---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2
FROM smelt.ref('payments')
GROUP BY is_verified
