-- Phase 29 fixture: demonstrate PASSING clause block-syntax for fragment-sort
-- parameters. Calls `session_rollup` with `metrics` supplied via a PASSING
-- block rather than as an inline argument.  This is the §10 block-syntax form
-- described in the research doc: instead of writing
--
--   smelt.fn.session_rollup(..., metrics => (COUNT(*)))
--
-- the caller writes:
--
--   smelt.fn.session_rollup(...) PASSING metrics AS (COUNT(*))
--
-- Phase 29 binds the PASSING body to the `metrics: SelectItems<Agg, sessionized>`
-- parameter by name and type-checks it identically to an inline argument.
SELECT *
FROM smelt.functions.session_rollup(
    smelt.sources.source.session_events,
    user_col => CAST('u' AS VARCHAR),
    ts_col   => CAST('2020-01-01' AS TIMESTAMP)
) PASSING metrics AS (COUNT(*)) AS sr

