---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT order_id, segment, is_verified
    FROM smelt.ref('sql_l1_86')
    WHERE is_active = true
),
aggregated AS (
    SELECT order_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY order_id
)
SELECT
    a.order_id,
    a.cnt,
    f.segment
FROM aggregated a
INNER JOIN filtered f ON a.order_id = f.order_id
