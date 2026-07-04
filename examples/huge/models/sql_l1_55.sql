---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    email_domain,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.page_views
GROUP BY email_domain
HAVING COUNT(*) > 10
