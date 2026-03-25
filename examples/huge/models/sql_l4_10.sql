---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT user_id, status, page_path
    FROM smelt.ref('sql_l3_231')
    WHERE quantity > 0
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.status
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
