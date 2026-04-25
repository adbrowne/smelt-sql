-- Phase 22 fixture: exercises session_rollup end-to-end.
-- The `metrics` and `filters` parameters use their defaults.
-- Phase 47: now projects the explicit session_rollup output columns
-- (rather than `*`) so the inferred return schema is verified.
SELECT
    sr.user_col,
    sr.session_id,
    sr.session_start,
    sr.session_end,
    sr.event_count
FROM smelt.fn.session_rollup(
    smelt.source('source.session_events'),
    user_col => CAST('u' AS VARCHAR),
    ts_col => CAST('2020-01-01' AS TIMESTAMP)
) AS sr
