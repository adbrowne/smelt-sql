---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    SUM(quantity) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.sql_l3_117
GROUP BY session_id

