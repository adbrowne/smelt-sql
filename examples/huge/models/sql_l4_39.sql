---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    AVG(amount) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.models.sql_l3_177
GROUP BY session_id
HAVING COUNT(*) > 10

