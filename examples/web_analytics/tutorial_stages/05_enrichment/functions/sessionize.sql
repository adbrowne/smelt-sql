-- Bounded sessionization. Assigns each event in `source` a stable session
-- identity, `session_start_ts` (the timestamp of the session's first event),
-- under the 30-minute inactivity + platform-boundary rule, with a
-- **clock-anchored cut**: a session rooted before 00:30 dies at its own
-- day's end; a session rooted at or after 00:30 may cross one midnight but
-- always dies at the *second* midnight. Every session spans at most two
-- calendar days (< 48h) — see
-- `docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"silver.sessions — clock-anchored cut" for the closed-form proof this
-- function implements.
--
-- Steps:
--   _marked    — LAG the previous event's ts/platform within the partition.
--   _bounded   — flag a *natural* session boundary (carry its ts) when the
--                gap exceeds 30 minutes, the platform changes, or there is
--                no predecessor.
--   _candidate — for each event, the most recent natural boundary at or
--                before it (running MAX), found in a trailing `max_lookback`
--                frame; NULL if none is in reach.
--   _deadlined — the candidate's deadline: `end_of_day(date(candidate))` if
--                the candidate's time-of-day is before 00:30, else
--                `end_of_day(date(candidate) + 1 day)`.
--   final      — `session_start_ts` is the candidate root if the event is
--                still within its deadline; otherwise the event is a
--                **forced root**, and its session_start_ts is the first
--                event of its own calendar day. A forced root is never
--                itself a natural boundary (no gap/platform break struck
--                it), so nothing marks it as a `_boundary_ts` for later
--                events to discover via `_candidate` — but no lookback
--                beyond the *current* day is needed to recover it, because
--                a forced root always lands before 00:30 of its own day
--                (the chain that reaches a deadline has, by construction,
--                only sub-30-minute gaps since its last accepted event, and
--                the deadline itself is a midnight, so the first breaching
--                event lands within 30 minutes after that midnight). This
--                is why the cut is window-independent: no arbitrarily long
--                history is ever required, only the current day plus a
--                bounded trailing lookback for natural boundaries.
--
-- The `RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW` frames are
-- the load-bearing lookback declaration, named `max_lookback` throughout
-- this function and its caller (`silver.sessions`): the planner derives this
-- bound from them (bound derivation runs on the expanded SQL, see
-- docs/specs/incremental_models.md), so a caller does not restate the
-- lookback. A window frame bound must be a literal `INTERVAL '...'` (the
-- grammar admits `UNBOUNDED` / `CURRENT ROW` / a number / an `INTERVAL`
-- literal — not a parameter reference), so the value is restated in each of
-- the three frames below and again in `silver.sessions`'s Form B filter and
-- its explicit `HAVING` cap assertion (which restates the *separate*,
-- numerically coincident max-session-span invariant, < 48h); all five
-- occurrences must agree. Output carries `_prev_ts` / `_prev_platform` /
-- `_boundary_ts` / `_candidate_root_ts` / `_deadline` bookkeeping columns;
-- callers reference only the columns they need.
smelt.define sessionize(
    source: TableExpr,
    partition_col: Expr<Integer>,
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
