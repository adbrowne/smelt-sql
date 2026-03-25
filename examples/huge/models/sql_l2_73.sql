---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    AVG(price) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('py_l1_345')
GROUP BY session_id
HAVING COUNT(*) > 10
