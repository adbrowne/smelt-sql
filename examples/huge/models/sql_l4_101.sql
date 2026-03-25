---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    SUM(amount) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    AVG(price) AS agg_2
FROM smelt.ref('py_l3_316')
GROUP BY event_date
