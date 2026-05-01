---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    page_path,
    MAX(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.models.sql_l3_126
GROUP BY page_path
HAVING COUNT(*) > 10

