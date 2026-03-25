---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('signups')
GROUP BY event_date
HAVING COUNT(*) > 10
