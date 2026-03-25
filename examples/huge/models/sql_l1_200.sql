---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    SUM(amount) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('categories')
GROUP BY device_type
HAVING COUNT(*) > 10
