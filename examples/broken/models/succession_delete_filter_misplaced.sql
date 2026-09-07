---
materialization: table
refresh: incremental
---
-- SuccessionDeleteFilterMisplaced: `QUALIFY is_deleted` is not exactly
-- `QUALIFY NOT <row-local boolean column>` (`docs/specs/incremental_shapes.md`
-- §"Delete events").
SELECT
    customer_id,
    changed_at,
    is_deleted,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at
FROM smelt.sources.succession_changes
QUALIFY is_deleted
