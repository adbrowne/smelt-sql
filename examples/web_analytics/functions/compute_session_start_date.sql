-- Project a `session_start_date` column derived from each row's session
-- partition.  For every row in `source`, the value is the DATE of the earliest
-- ts_col observed within (partition_col, session_seq_col).  Output extends
-- each input row with the new column; existing columns are passed through.
--
-- Lives in its own transparent function so that the FIRST_VALUE OVER window
-- expression is hidden from the planner's outer-body safety scan.  Callers
-- (notably models/silver/sessions.sql) can then declare `incremental: enabled`
-- without the planner downgrading the model to a full rebuild on account of
-- the `OVER` clause in their outer body.
smelt.define compute_session_start_date(
    source: TableExpr,
    partition_col: Expr<Integer>,
    session_seq_col: Expr<BigInt>,
    ts_col: Expr<Date>
) -> TableExpr AS (
    SELECT
        *,
        CAST(FIRST_VALUE(ts_col) OVER (PARTITION BY partition_col, session_seq_col ORDER BY ts_col) AS DATE) AS session_start_date
    FROM source
)
