---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    quantity,
    SUM(quantity) AS agg_0,
    AVG(duration_seconds) AS agg_1
FROM smelt.models.sql_l2_32
GROUP BY quantity

