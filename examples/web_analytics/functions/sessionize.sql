-- Assign a session_seq to each event in `source`, partitioned by partition_col
-- and ordered by ts_col. A session boundary fires when either the inactivity
-- gap exceeds `gap` OR the platform column changes between consecutive events.
-- The output schema extends `source.*` with a single `session_seq: BIGINT`
-- column added by the explicit projection (per the TableExpr return-schema
-- inference rule in docs/specs/functions.md).
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Timestamp>,
    platform_col: Expr<Text>,
    gap: Expr<Interval> = INTERVAL '30 minutes'
) -> TableExpr AS (
    SELECT
        source.*,
        SUM(
            CASE
                WHEN ts_col - LAG(ts_col) OVER (PARTITION BY partition_col ORDER BY ts_col) > gap
                  OR LAG(platform_col) OVER (PARTITION BY partition_col ORDER BY ts_col) != platform_col
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY partition_col ORDER BY ts_col) AS session_seq
    FROM source
)
