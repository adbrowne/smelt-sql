---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT campaign_id, is_active, segment
    FROM smelt.ref('sql_l3_167')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT campaign_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY campaign_id
)
SELECT
    a.campaign_id,
    a.cnt,
    f.is_active
FROM aggregated a
INNER JOIN filtered f ON a.campaign_id = f.campaign_id
