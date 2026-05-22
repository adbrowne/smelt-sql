---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    AVG(amount) AS val_1,
    AVG(price) AS val_2
FROM smelt.sql_l1_223
GROUP BY email_domain
HAVING COUNT(*) > 10
