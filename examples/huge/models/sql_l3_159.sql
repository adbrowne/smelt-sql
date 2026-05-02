---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    MAX(created_at) AS val_1,
    COUNT(*) AS val_2
FROM smelt.models.sql_l2_171
GROUP BY price
HAVING COUNT(*) > 10

