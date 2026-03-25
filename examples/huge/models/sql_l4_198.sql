---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_time,
    AVG(duration_seconds) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('sql_l3_90')
GROUP BY event_time
HAVING COUNT(*) > 10
