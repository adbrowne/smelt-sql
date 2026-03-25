---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    campaign_id,
    channel,
    browser
FROM smelt.ref('py_l3_287')
WHERE category IS NOT NULL
