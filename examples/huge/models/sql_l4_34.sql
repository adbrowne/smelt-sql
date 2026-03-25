---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT country, campaign_id, status
    FROM smelt.ref('py_l3_315')
    WHERE platform = 'web'
),
aggregated AS (
    SELECT country, COUNT(*) AS cnt
    FROM filtered
    GROUP BY country
)
SELECT
    a.country,
    a.cnt,
    f.campaign_id
FROM aggregated a
INNER JOIN filtered f ON a.country = f.country
