-- Assign a session_seq counter to each row in `source`, partitioned by
-- partition_col and ordered by ts_col.  A session boundary fires when either
-- the inactivity gap between consecutive events exceeds `gap` microseconds OR
-- the platform_col value changes between consecutive events.
--
-- The output extends each row from `source` with a session_seq BIGINT column
-- and two internal bookkeeping columns (_smelt_prev_ts_us, _smelt_prev_platform)
-- used during expansion; callers should reference only the columns they need.
--
-- The `gap` parameter is in microseconds (epoch_us units).  The default is
-- 30 minutes expressed as 30 * 60 * 1 000 000 = 1 800 000 000 μs.
--
-- Using epoch_us() arithmetic ensures the comparison is BIGINT - BIGINT,
-- which works whether ts_col is stored as TIMESTAMP or DATE (DuckDB stores
-- DATE values as epoch-day integers, and epoch_us(DATE) is well-defined).
--
-- A CTE (_lagged) is required because DuckDB prohibits nested window
-- functions: the LAG calls are resolved in _lagged before the SUM
-- session counter is computed in the outer SELECT.
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Date>,
    platform_col: Expr<Text>,
    gap: Expr<BigInt> = 30 * 60 * 1000000
) -> TableExpr AS (
    WITH _lagged AS (
        SELECT
            *,
            LAG(epoch_us(ts_col)) OVER (PARTITION BY partition_col ORDER BY ts_col) AS _smelt_prev_ts_us,
            LAG(platform_col) OVER (PARTITION BY partition_col ORDER BY ts_col) AS _smelt_prev_platform
        FROM source
    )
    SELECT
        *,
        SUM(
            CASE
                WHEN epoch_us(ts_col) - _smelt_prev_ts_us > gap
                  OR _smelt_prev_platform != platform_col
                THEN 1
                ELSE 0
            END
        ) OVER (PARTITION BY partition_col ORDER BY ts_col) AS session_seq
    FROM _lagged
)
