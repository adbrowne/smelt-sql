---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    page_path,
    created_at
FROM smelt.categories
WHERE user_id IN (
    SELECT user_id FROM smelt.categories WHERE quantity > 0
)
