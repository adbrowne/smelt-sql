---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT plan_type, discount, session_id
    FROM smelt.ref('sql_l3_244')
    WHERE quantity > 0
)
SELECT
    b.plan_type,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_121') j ON b.user_id = j.user_id
GROUP BY b.plan_type
