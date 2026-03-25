---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    MAX(created_at) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('refunds')
GROUP BY event_time
HAVING COUNT(*) > 10
