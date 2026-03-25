---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    AVG(amount) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('sql_l3_180')
GROUP BY ip_address
HAVING COUNT(*) > 10
