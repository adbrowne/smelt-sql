---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT segment, event_date, revenue
    FROM smelt.ref('sql_l1_145')
    WHERE event_type = 'purchase'
)
SELECT
    b.segment,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_145') j ON b.user_id = j.user_id
GROUP BY b.segment
