---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT device_type, event_type, quantity
    FROM smelt.ref('py_l1_269')
    WHERE country = 'US'
)
SELECT
    b.device_type,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_431') j ON b.user_id = j.user_id
GROUP BY b.device_type
