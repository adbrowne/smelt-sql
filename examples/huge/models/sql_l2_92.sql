---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('py_l1_491')
GROUP BY referrer
HAVING COUNT(*) > 10
