-- Phase 44b fixture: §10 block-syntax composition — an outer function
-- wraps `session_rollup` and forwards its own `metrics` and `alerts`
-- fragment parameters through PASSING clauses.
--
-- This demonstrates the bare `smelt.fn.*` CTE body syntax:
--
--   WITH base AS (
--       smelt.fn.session_rollup(source, user_col, ts_col)
--       PASSING metrics AS (metrics)
--   )
--
-- The inner `metrics` reference in `PASSING metrics AS (metrics)` is a
-- bare reference to `outer_wrapper`'s own `metrics: SelectItems<Agg>`
-- parameter — Phase 44b allows this without emitting FragmentKindMismatch.
smelt.define monitored_session_rollup(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    metrics: SelectItems<Agg> = (),
    alerts: SelectItems<Agg, base> = ()
) -> TableExpr AS (
    WITH base AS (
        smelt.functions.session_rollup(source, user_col, ts_col)
        PASSING metrics AS (metrics)
    )
    SELECT base.*,
        alerts
    FROM base
)

