---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    COUNT(*) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('sql_l3_160')
GROUP BY amount
HAVING COUNT(*) > 10
