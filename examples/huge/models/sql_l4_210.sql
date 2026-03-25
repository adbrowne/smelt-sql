---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('py_l3_437')
GROUP BY referrer
HAVING COUNT(*) > 10
