---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT user_id, category, plan_type
    FROM smelt.ref('sql_l3_185')
    WHERE score >= 50
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.category
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
