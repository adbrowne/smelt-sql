---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT os_name, device_type, campaign_id
    FROM smelt.sql_l1_180
    WHERE is_active = true
),
aggregated AS (
    SELECT os_name, COUNT(*) AS cnt
    FROM filtered
    GROUP BY os_name
)
SELECT
    a.os_name,
    a.cnt,
    f.device_type
FROM aggregated a
INNER JOIN filtered f ON a.os_name = f.os_name

