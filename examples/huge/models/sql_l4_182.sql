---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('py_l3_263')
GROUP BY rating
HAVING COUNT(*) > 10
