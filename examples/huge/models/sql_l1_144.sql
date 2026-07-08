---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT device_type, category, page_path
    FROM smelt.orders
    WHERE is_active = true
),
aggregated AS (
    SELECT device_type, COUNT(*) AS cnt
    FROM filtered
    GROUP BY device_type
)
SELECT
    a.device_type,
    a.cnt,
    f.category
FROM aggregated a
INNER JOIN filtered f ON a.device_type = f.device_type
