---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT user_id, segment, order_id
    FROM smelt.ref('py_l3_375')
    WHERE amount > 0
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.segment
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
