-- Phase 17 fixture: the canonical `sessionize` function from
-- research §3. Takes a `TableExpr` source plus column references and
-- an interval gap (with a default). Body uses `LAG()` and
-- `SUM() OVER (…)` in SELECT-list position — both synthesise
-- `ExprKind::Window`, which must NOT trigger the Phase-14
-- WindowInScalarContext check (that only fires in WHERE / GROUP BY).
--
-- The output schema is inferred at call sites via
-- `infer_tableexpr_return_schema`: `source.*` expands to the caller's
-- bound schema and `session_id: BigInt` is added from the explicit
-- projection.
smelt.define sessionize(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT
        source.*,
        SUM(CASE WHEN ts_col - LAG(ts_col) OVER (PARTITION BY user_col ORDER BY ts_col) > gap THEN 1 ELSE 0 END)
            OVER (PARTITION BY user_col ORDER BY ts_col) AS session_id
    FROM source
)

