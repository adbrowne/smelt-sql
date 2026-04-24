-- Phase 22 fixture: exercises session_rollup end-to-end.
-- The `metrics` and `filters` parameters use their defaults.
SELECT *
FROM smelt.fn.session_rollup(
    smelt.source('source.session_events'),
    user_col => CAST('u' AS VARCHAR),
    ts_col => CAST('2020-01-01' AS TIMESTAMP)
) AS sr
