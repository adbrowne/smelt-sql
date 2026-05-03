---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT device_type, is_verified, product_id
    FROM smelt.sql_l2_138
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT device_type, COUNT(*) AS cnt
    FROM filtered
    GROUP BY device_type
)
SELECT
    a.device_type,
    a.cnt,
    f.is_verified
FROM aggregated a
INNER JOIN filtered f ON a.device_type = f.device_type

