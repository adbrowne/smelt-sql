---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT user_id, revenue, email_domain
    FROM smelt.ref('invoices')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.revenue
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
