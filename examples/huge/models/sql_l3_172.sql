---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT device_type, is_verified, product_id
    FROM smelt.ref('py_l2_382')
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
