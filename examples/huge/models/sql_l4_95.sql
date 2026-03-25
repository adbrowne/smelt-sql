---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT ip_address, updated_at, email_domain
    FROM smelt.ref('py_l3_364')
    WHERE status = 'active'
)
SELECT
    b.ip_address,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_440') j ON b.user_id = j.user_id
GROUP BY b.ip_address
