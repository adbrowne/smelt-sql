-- Phase 44b fixture: call `monitored_session_rollup` from a model,
-- demonstrating that the fixture function is callable with PASSING clauses.
SELECT *
FROM smelt.functions.monitored_session_rollup(
    smelt.sources.source.session_events,
    user_col => CAST('u' AS VARCHAR),
    ts_col   => CAST('2020-01-01' AS TIMESTAMP)
) PASSING metrics AS (COUNT(*))

