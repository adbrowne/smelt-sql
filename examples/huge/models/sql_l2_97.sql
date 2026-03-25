---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_date, page_path, ip_address
    FROM smelt.ref('py_l1_391')
    WHERE event_type = 'purchase'
)
SELECT
    b.event_date,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_477') j ON b.user_id = j.user_id
GROUP BY b.event_date
