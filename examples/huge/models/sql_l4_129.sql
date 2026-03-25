---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT email_domain, cohort_date, amount
    FROM smelt.ref('sql_l3_243')
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.email_domain,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_75') j ON b.user_id = j.user_id
GROUP BY b.email_domain
