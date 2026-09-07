---
materialization: table
refresh: incremental
---
SELECT
  customer_id,
  changed_at,
  tier,
  LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS valid_to
FROM smelt.sources.customer_changes
