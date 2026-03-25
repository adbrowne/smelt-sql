---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    SUM(quantity) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('py_l3_396')
GROUP BY ip_address
HAVING COUNT(*) > 10
