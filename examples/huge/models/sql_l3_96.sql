---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    COUNT(*) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2
FROM smelt.models.sql_l2_164
GROUP BY event_date

