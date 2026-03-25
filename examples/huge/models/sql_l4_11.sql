---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    SUM(revenue) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('sql_l3_87')
GROUP BY device_type
HAVING COUNT(*) > 10
