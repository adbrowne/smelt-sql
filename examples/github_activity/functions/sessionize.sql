-- Bounded sessionization, ported from `examples/web_analytics/functions/
-- sessionize.sql` (project isolation means each smelt project owns its own
-- copy — `docs/specs/architecture.md` §"Project isolation rule"). Assigns
-- each event in `source` a stable session identity, `session_start_ts` (the
-- timestamp of the session's first event), under the 30-minute inactivity
-- rule, with a **clock-anchored cut**: a session rooted before 00:30 dies at
-- its own day's end; a session rooted at or after 00:30 may cross one
-- midnight but always dies at the *second* midnight. Every session spans at
-- most two calendar days (< 48h) — see `docs/research/
-- 20260711-clock-vs-root-anchored-sessions.md` §"silver.sessions —
-- clock-anchored cut" for the closed-form proof this function implements.
--
-- `platform_col` is kept from the web_analytics original for shape parity
-- even though GitHub activity has no platform axis; callers here pass a
-- constant so that arm of the boundary rule never fires (`NULL != NULL` is
-- `NULL`, never `TRUE`).
--
-- The `RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW` frames are
-- the load-bearing lookback declaration, named `max_lookback` throughout
-- this function and its caller (`silver.actor_sessions`): the planner
-- derives this bound from them, so a caller does not restate the lookback.
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<BigInt>,
    ts_col: Expr<Timestamp>,
    platform_col: Expr<Text>
) -> TableExpr AS (
    WITH _marked AS (
        SELECT
            *,
            LAG(ts_col) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
            ) AS _prev_ts,
            LAG(platform_col) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
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
    ),
    _candidate AS (
        SELECT
            *,
            MAX(_boundary_ts) OVER (
                PARTITION BY partition_col ORDER BY ts_col
                RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
            ) AS _candidate_root_ts
        FROM _bounded
    ),
    _deadlined AS (
        SELECT
            *,
            CASE
                WHEN _candidate_root_ts IS NULL THEN NULL
                WHEN CAST(_candidate_root_ts AS TIME) < TIME '00:30:00'
                    THEN CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '1 day'
                ELSE CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '2 days'
            END AS _deadline
        FROM _candidate
    )
    SELECT
        *,
        CASE
            WHEN _candidate_root_ts IS NOT NULL AND ts_col < _deadline THEN _candidate_root_ts
            ELSE MIN(ts_col) OVER (PARTITION BY partition_col, CAST(ts_col AS DATE))
        END AS session_start_ts
    FROM _deadlined
)
