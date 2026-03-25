---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    SUM(revenue) AS agg_0,
    AVG(price) AS agg_1
FROM smelt.ref('sql_l1_5')
GROUP BY device_type
