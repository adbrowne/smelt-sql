---
materialization: table
refresh: incremental
---
-- SuccessionPartitionKeyMismatch: the two succession windows partition by
-- different key sets (`customer_id` vs `name`)
-- (`docs/specs/model_properties.md` §"Keyed-succession classification").
SELECT
    customer_id,
    changed_at,
    name,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at,
    LAG(changed_at) OVER (PARTITION BY name ORDER BY changed_at) AS prev_changed_at
FROM smelt.sources.succession_changes
