---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT device_type, referrer, revenue
    FROM smelt.ref('py_l2_327')
    WHERE country = 'US'
),
aggregated AS (
    SELECT device_type, COUNT(*) AS cnt
    FROM filtered
    GROUP BY device_type
)
SELECT
    a.device_type,
    a.cnt,
    f.referrer
FROM aggregated a
INNER JOIN filtered f ON a.device_type = f.device_type
