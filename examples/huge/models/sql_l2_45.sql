---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT email_domain, page_path, tier
    FROM smelt.ref('sql_l1_228')
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT email_domain, COUNT(*) AS cnt
    FROM filtered
    GROUP BY email_domain
)
SELECT
    a.email_domain,
    a.cnt,
    f.page_path
FROM aggregated a
INNER JOIN filtered f ON a.email_domain = f.email_domain
