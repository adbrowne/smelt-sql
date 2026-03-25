---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('logs')
GROUP BY user_id
HAVING COUNT(*) > 10
