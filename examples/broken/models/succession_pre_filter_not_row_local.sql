---
materialization: table
refresh: incremental
---
-- SuccessionPreFilterNotRowLocal: the pre-window filter calls `NOW()`, a
-- run-nondeterministic function, so it is not a deterministic row-local
-- predicate (`docs/specs/incremental_shapes.md` §"Run shape and late
-- events").
SELECT
    customer_id,
    changed_at,
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at
FROM smelt.sources.succession_changes
WHERE changed_at <= NOW()
