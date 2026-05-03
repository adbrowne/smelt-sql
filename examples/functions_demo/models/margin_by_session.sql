-- Phase 18 end-to-end pipeline: chains add_margin → sessionize.
-- Demonstrates that Phase 17's TableExpr return-schema inference threads
-- correctly into a downstream smelt.fn.* call's FROM scope.
--
-- Note: projects only `session_id` (a column added by sessionize itself)
-- rather than pass-through columns from add_margin's output (e.g. `margin`).
-- Schema propagation for nested smelt.fn.* TableExpr arguments is deferred
-- to a later phase — see "Deferred during implementation" in the plan.
SELECT session_id
FROM smelt.functions.sessionize(
    smelt.functions.add_margin(smelt.orders),
    user_col => CAST('' AS VARCHAR),
    ts_col => CAST('2020-01-01' AS TIMESTAMP)
) AS s

