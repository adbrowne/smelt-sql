---
materialization: table
refresh: incremental
---
-- SuccessionRowLocalColumnViolation: `COUNT(*)` is a projected non-window
-- column that is itself an aggregate, not row-local
-- (`docs/specs/model_properties.md` §"Keyed-succession classification").
SELECT
    customer_id,
    changed_at,
    COUNT(*) AS n,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at
FROM smelt.sources.succession_changes
