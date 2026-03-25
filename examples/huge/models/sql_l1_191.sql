---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    COUNT(*) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('refunds')
GROUP BY country
HAVING COUNT(*) > 10
