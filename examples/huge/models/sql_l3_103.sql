---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    tier,
    event_date,
    page_path,
    transaction_id
FROM smelt.ref('sql_l2_83')
WHERE platform = 'web'
