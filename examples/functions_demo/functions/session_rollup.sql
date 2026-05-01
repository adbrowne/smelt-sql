-- Phase 22 fixture: full session_rollup (research §6).
-- `metrics: SelectItems<Agg, sessionized>` references the CTE name
-- `sessionized` defined in the body's WITH clause — Phase 22 extends
-- `unknown_context_diagnostics_for_file` to accept CTE names as valid
-- context references, and extends `check_fragment_context_bindings` to
-- resolve column sets via `is_cte` / `cte_columns`.
smelt.define session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes',
    metrics: SelectItems<Agg, sessionized> = (),
    filters: Expr<Boolean> = TRUE
) -> TableExpr AS (
    WITH sessionized AS (
        SELECT * FROM smelt.functions.sessionize(source, user_col, ts_col, gap)
    )
    SELECT
        user_col, session_id,
        MIN(ts_col) AS session_start, MAX(ts_col) AS session_end,
        COUNT(*) AS event_count,
        metrics
    FROM sessionized
    WHERE filters
    GROUP BY user_col, session_id
)

