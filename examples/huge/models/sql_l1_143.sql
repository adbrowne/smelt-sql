---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT os_name, session_id, is_active
    FROM smelt.shipments
    WHERE country = 'US'
)
SELECT
    b.os_name,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.shipments j ON b.user_id = j.user_id
GROUP BY b.os_name
