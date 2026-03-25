---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    MAX(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l1_126')
GROUP BY event_time
HAVING COUNT(*) > 10
