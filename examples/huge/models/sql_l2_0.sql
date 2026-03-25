---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT os_name, device_type, campaign_id
    FROM smelt.ref('py_l1_366')
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
