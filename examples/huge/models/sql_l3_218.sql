---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    MIN(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.sql_l2_102
GROUP BY email_domain
HAVING COUNT(*) > 10

