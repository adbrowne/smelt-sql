---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('notifications')
GROUP BY user_id
HAVING COUNT(*) > 10
