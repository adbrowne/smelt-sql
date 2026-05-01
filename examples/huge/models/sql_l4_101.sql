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
    SUM(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2
FROM smelt.models.sql_l3_234
GROUP BY event_date

