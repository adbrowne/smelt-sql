---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT user_id, profit, campaign_id
    FROM smelt.ref('invoices')
    WHERE platform = 'web'
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.profit
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
