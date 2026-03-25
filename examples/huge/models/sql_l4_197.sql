---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_type, page_path, campaign_id
    FROM smelt.ref('py_l3_420')
    WHERE quantity > 0
)
SELECT
    b.event_type,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_261') j ON b.user_id = j.user_id
GROUP BY b.event_type
