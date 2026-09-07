---
materialization: table
refresh: incremental
---
-- SuccessionIdentityNotProjected: the key column `customer_id` is used in
-- PARTITION BY but never projected row-locally, so the derived (k, t)
-- identity cannot be recovered from the presented table
-- (`docs/specs/model_properties.md` §"Keyed-succession classification").
SELECT
    changed_at,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at
FROM smelt.sources.succession_changes
