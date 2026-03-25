---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    email_domain,
    event_type
FROM smelt.ref('sql_l3_223')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_223') WHERE status = 'active'
)
