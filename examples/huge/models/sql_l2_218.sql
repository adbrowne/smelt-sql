---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT country, campaign_id, is_active
    FROM smelt.ref('py_l1_382')
    WHERE platform = 'web'
)
SELECT
    b.country,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_48') j ON b.user_id = j.user_id
GROUP BY b.country
