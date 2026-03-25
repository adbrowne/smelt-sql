---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    MAX(created_at) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('refunds')
GROUP BY platform
HAVING COUNT(*) > 10
