---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    AVG(price) AS agg_0,
    SUM(amount) AS agg_1,
    AVG(amount) AS agg_2,
    COUNT(*) AS agg_3,
    SUM(quantity) AS agg_4
FROM smelt.models.sql_l3_129
GROUP BY event_type

