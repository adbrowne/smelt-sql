---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    AVG(price) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1
FROM smelt.ref('py_l1_452')
GROUP BY device_type
