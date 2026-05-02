-- Phase 17 fixture: `sessionize` end-to-end. The outer SELECT
-- projects `session_user` (from `source.*`) and `session_id` (from
-- the body's explicit projection) through the inferred TableExpr
-- return schema. Phase 17's `RefSchemaProvider::smelt_fn_columns`
-- seeds the caller's FROM-scope so these columns resolve without
-- any wildcard fallback.
--
-- `user_col` / `ts_col` are passed as CAST-literals — Phase 19's
-- context-binding syntax will replace these with column references
-- once that phase lands.
SELECT session_user, session_id
FROM smelt.functions.sessionize(
    smelt.sources.source.session_events,
    user_col => CAST('u' AS VARCHAR),
    ts_col => CAST('2020-01-01' AS TIMESTAMP)
) AS s

