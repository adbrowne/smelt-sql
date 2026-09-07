---
materialization: table
refresh: incremental
---
-- SuccessionPreFilterNegatesFlag (Warning, advisory): a bare negated
-- boolean pre-window filter is admitted, but never closes its predecessor
-- — `QUALIFY NOT is_deleted` is the suggested fix
-- (`docs/specs/incremental_shapes.md` §"Delete events"). Recognized, no
-- Error diagnostic.
SELECT
    customer_id,
    changed_at,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at
FROM smelt.sources.succession_changes
WHERE NOT is_deleted
