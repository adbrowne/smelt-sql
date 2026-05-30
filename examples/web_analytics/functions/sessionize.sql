-- Bounded sessionization. Assigns each event in `source` a stable session
-- identity, `session_start_ts` (the timestamp of the session's first event),
-- under the 30-minute inactivity + platform-boundary rule, reconstructed across
-- midnight from a bounded 1-day lookback.
--
-- Steps:
--   _marked  — LAG the previous event's ts/platform within the partition.
--   _bounded — flag a session boundary (carry its ts) when the gap exceeds 30
--              minutes, the platform changes, or there is no predecessor.
--   final    — each event's session identity is the most recent boundary ts at
--              or before it (running MAX); events with no in-frame boundary
--              fall back to their own ts (the cap).
--
-- The `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW` frames are the
-- load-bearing lookback declaration. They live inside this function body, yet
-- the planner derives the 1-day bound from them — bound derivation runs on the
-- expanded SQL (see docs/specs/incremental_models.md) — so a caller does not
-- restate the lookback. The frame is also the session-length cap. Output carries
-- `_prev_ts` / `_prev_platform` / `_boundary_ts` bookkeeping columns; callers
-- reference only the columns they need.
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
    ts_col: Expr<Date>,
    platform_col: Expr<Text>
) -> TableExpr AS (
    WITH _marked AS (
        SELECT
            *,
            LAG(ts_col) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ) AS _prev_ts,
            LAG(platform_col) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ) AS _prev_platform
        FROM source
    ),
    _bounded AS (
        SELECT
            *,
            CASE
                WHEN _prev_ts IS NULL THEN ts_col
                WHEN epoch_us(ts_col) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN ts_col
                WHEN _prev_platform != platform_col THEN ts_col
                ELSE NULL
            END AS _boundary_ts
        FROM _marked
    )
    SELECT
        *,
        COALESCE(
            MAX(_boundary_ts) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ),
            ts_col
        ) AS session_start_ts
    FROM _bounded
)
