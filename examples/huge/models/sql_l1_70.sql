---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('notifications')
GROUP BY ip_address
HAVING COUNT(*) > 10
