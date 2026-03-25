---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    AVG(amount) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('subscriptions')
GROUP BY platform
HAVING COUNT(*) > 10
