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
    discount,
    AVG(amount) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.page_views
GROUP BY discount
HAVING COUNT(*) > 10

