---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT browser, channel, user_id
    FROM smelt.ref('py_l1_389')
    WHERE event_type = 'purchase'
)
SELECT
    b.browser,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_223') j ON b.user_id = j.user_id
GROUP BY b.browser
