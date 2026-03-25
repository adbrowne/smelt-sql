---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    AVG(amount) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('py_l2_442')
GROUP BY plan_type
HAVING COUNT(*) > 10
