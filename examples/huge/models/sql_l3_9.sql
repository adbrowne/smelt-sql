---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT email_domain, updated_at, score
    FROM smelt.ref('py_l2_416')
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT email_domain, COUNT(*) AS cnt
    FROM filtered
    GROUP BY email_domain
)
SELECT
    a.email_domain,
    a.cnt,
    f.updated_at
FROM aggregated a
INNER JOIN filtered f ON a.email_domain = f.email_domain
