---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    COUNT(*) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2
FROM smelt.ref('py_l2_258')
GROUP BY event_date
