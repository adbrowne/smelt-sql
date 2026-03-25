---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    AVG(amount) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('py_l3_315')
GROUP BY session_id
HAVING COUNT(*) > 10
