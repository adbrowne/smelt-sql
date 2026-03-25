---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT session_id, tier, created_at
    FROM smelt.ref('sql_l2_34')
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT session_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY session_id
)
SELECT
    a.session_id,
    a.cnt,
    f.tier
FROM aggregated a
INNER JOIN filtered f ON a.session_id = f.session_id
