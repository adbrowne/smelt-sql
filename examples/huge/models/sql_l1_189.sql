---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    MIN(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('clicks')
GROUP BY is_active
HAVING COUNT(*) > 10
