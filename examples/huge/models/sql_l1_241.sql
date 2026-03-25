---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('invoices')
GROUP BY event_date
HAVING COUNT(*) > 10
