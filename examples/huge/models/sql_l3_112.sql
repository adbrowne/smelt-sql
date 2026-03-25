---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT channel, profit, category
    FROM smelt.ref('sql_l2_36')
    WHERE category IS NOT NULL
)
SELECT
    b.channel,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_393') j ON b.user_id = j.user_id
GROUP BY b.channel
