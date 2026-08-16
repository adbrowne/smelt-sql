---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
# The local gap/platform-boundary detection windows partition by device_id,
# not session_start_date — the analyzer cannot statically prove they are
# partition-aligned (same shape as `silver/events_deduped.sql`'s own
# redelivery-dedup window). It is safe in practice: each window's own
# `_local_root_ts` output feeds only the epoch-bucketing and self-read gate
# below, both scoped to that same event's own row, never chunked across a
# device boundary.
#
# The self-read's five correlated scalar subqueries (`_with_self`) each end
# in `LIMIT 1` — not a global top-k (the analyzer's own concern: a LIMIT
# whose row subset would differ across time ranges), but a per-row
# correlated lookup picking the single most-recently-ended open session for
# THIS event's own device — deterministic and identical regardless of how
# the run window is chunked.
safety_overrides:
  allow_window_functions: true
  allow_limit: true
---
-- One row per session under the same 30-minute inactivity + platform-boundary
-- rule as `silver.sessions`, but with the **root-anchored cut**: a day-D
-- event continues an open session only if that session rooted less than two
-- calendar days ago; otherwise it strikes a new root. Unlike the clock
-- table, the cutoff's phase is inherited from the session's own history —
-- no bounded read of source events alone can recover it, so this model reads
-- its own prior output (`smelt.silver.sessions_chained`, the self-reference
-- below) to learn which sessions are still open and when they rooted. See
-- `docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"silver.sessions_chained — root-anchored cut" for the design and
-- `docs/specs/incremental_shapes.md` §"Window independence and self-referential
-- models" for why a backward-bounded (no forward reach) self-read proves
-- `Ordered`: the planner forces this model's partitions to build strictly in
-- temporal order, one at a time — backfills of this table cannot be
-- parallelised, which is the whole teaching point of the divergence from
-- `silver.sessions`.
--
-- Inline SQL, no reusable transparent function: `smelt.functions.sessionize`
-- assigns a session root from a bounded *window frame over source events
-- alone* (window-independent by construction); a root-anchored cut needs the
-- self-reference's carried state (is there an open session for this device,
-- when did it root, when did it last see an event) folded in, which doesn't
-- factor into that shape.
--
-- session_id is (device_id, session_start_ts) — stable across run windows,
-- same identity scheme as `silver.sessions`.
WITH events AS (
    SELECT device_id, event_ts, event_date, platform, utm_campaign
    FROM smelt.silver.events_deduped
),
-- Local (in-batch) natural-boundary detection: identical gap/platform rule
-- to `sessionize`'s own `_marked`/`_bounded`/candidate steps, but with no
-- clock deadline — a natural boundary here is struck only by the 30-minute
-- inactivity or platform-change rule, never by a periodic midnight
-- alignment. `_local_root_ts` is a purely local grouping key (this batch's
-- own events only); the root-anchored 2-day cutoff is layered on top below,
-- independently of this grouping.
_marked AS (
    SELECT
        *,
        LAG(event_ts) OVER (
            PARTITION BY device_id ORDER BY event_ts
            RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
        ) AS _prev_ts,
        LAG(platform) OVER (
            PARTITION BY device_id ORDER BY event_ts
            RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
        ) AS _prev_platform
    FROM events
),
_bounded AS (
    SELECT
        *,
        CASE
            WHEN _prev_ts IS NULL THEN event_ts
            WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
            WHEN _prev_platform != platform THEN event_ts
            ELSE NULL
        END AS _boundary_ts
    FROM _marked
),
-- `_local_root_ts` is a running "last non-null" carry-forward: once a
-- natural boundary is struck, it stays the candidate root for every later
-- row until a NEWER boundary supersedes it — with NO time cap. A `RANGE ...
-- INTERVAL` frame here (matching `_marked`'s own, purely declarative,
-- lookback-margin frames) would silently forget the candidate once a row is
-- more than that interval past it, producing a spurious "no candidate found"
-- gap in the middle of an otherwise-continuous chain whenever one batch's
-- own read window happens to span longer than the interval (unlike
-- `sessionize`'s clock-anchored candidate, which self-limits via the
-- midnight deadline and never needs to reach further than its own declared
-- margin). A `ROWS`-based frame carries no time semantics, so it does not
-- feed the analyzer's INTERVAL-based margin derivation (`_marked`'s frames
-- already declare the model's admitted lookback for that purpose).
_candidate AS (
    SELECT
        *,
        MAX(_boundary_ts) OVER (
            PARTITION BY device_id ORDER BY event_ts
            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
        ) AS _local_root_ts
    FROM _bounded
),
-- Self-read: does an already-open session exist for this device, rooted
-- less than two calendar days before this local chain's own root candidate?
-- Backward-bounded, 2-day reach, no forward reach at all (`after ==
-- Seconds::ZERO`) — the shape `window_independence` proves `Ordered` from.
-- "Root within two days" is the join bound; whether it actually continues
-- (an exact 30-minute gap from the open session's own last-known event) is
-- decided per event below, since a window-frame bound cannot express an
-- inequality that depends on another table's stored column.
--
-- No table alias on the self-reference, and correlated scalar subqueries
-- rather than a join: a `smelt.<path>` reference carrying an alias, or one
-- immediately followed by a `.column`/`.* ` accessor, does not round-trip
-- through the test framework's CTE-isolation printer
-- (`docs/specs/testing.md` §"CTE isolation") — it reprints a FROM item from
-- its AST, and a bare `smelt.<path>` token followed by anything other than a
-- clause boundary is mis-attributed. A correlated scalar subquery whose own
-- (sole) FROM target is the bare, alias-free self-reference, with its own
-- columns referenced unqualified (resolved against that lone FROM target)
-- and the outer row reached via `_candidate`'s own bare CTE name, sidesteps
-- both hazards. This is purely a printer-compatibility precaution; the
-- production `smelt build`/`smelt run` path (text-range substitution, not
-- AST reprinting) is unaffected either way. The five subqueries repeat the
-- same WHERE/ORDER BY/LIMIT because a window-independent one-shot lookup has
-- no shared CTE to factor them into without reintroducing an alias.
_with_self AS (
    SELECT
        _candidate.device_id,
        _candidate.event_ts,
        _candidate.event_date,
        _candidate.platform,
        _candidate.utm_campaign,
        _candidate._local_root_ts,
        (
            SELECT session_start_ts FROM smelt.silver.sessions_chained
            WHERE device_id = _candidate.device_id
                AND platform = _candidate.platform
                AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
            ORDER BY session_end DESC LIMIT 1
        ) AS _open_root_ts,
        (
            SELECT session_start FROM smelt.silver.sessions_chained
            WHERE device_id = _candidate.device_id
                AND platform = _candidate.platform
                AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
            ORDER BY session_end DESC LIMIT 1
        ) AS _open_session_start,
        (
            SELECT event_count FROM smelt.silver.sessions_chained
            WHERE device_id = _candidate.device_id
                AND platform = _candidate.platform
                AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
            ORDER BY session_end DESC LIMIT 1
        ) AS _open_event_count,
        (
            SELECT utm_campaign FROM smelt.silver.sessions_chained
            WHERE device_id = _candidate.device_id
                AND platform = _candidate.platform
                AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
            ORDER BY session_end DESC LIMIT 1
        ) AS _open_utm_campaign,
        (
            SELECT session_end FROM smelt.silver.sessions_chained
            WHERE device_id = _candidate.device_id
                AND platform = _candidate.platform
                AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
            ORDER BY session_end DESC LIMIT 1
        ) AS _open_session_end
    FROM _candidate
),
-- Anchor: the root this local chain would inherit if it continues an open
-- self session (exact 30-minute gap from that session's last known event),
-- else the local chain's own candidate root. This is a per-*local-chain*
-- constant (same for every row sharing `_local_root_ts`), not yet
-- age-checked.
_anchored AS (
    SELECT
        *,
        CASE
            WHEN _open_root_ts IS NOT NULL
                 AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
            THEN _open_root_ts
            ELSE _local_root_ts
        END AS _anchor_ts,
        CASE
            WHEN _open_root_ts IS NOT NULL
                 AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
            THEN _open_session_start
        END AS _anchor_seed_session_start,
        CASE
            WHEN _open_root_ts IS NOT NULL
                 AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
            THEN _open_event_count
        END AS _anchor_seed_event_count,
        CASE
            WHEN _open_root_ts IS NOT NULL
                 AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
            THEN _open_utm_campaign
        END AS _anchor_seed_utm_campaign
    FROM _with_self
),
-- Root-anchored 2-day cutoff, applied without recursion: a natural
-- (gap/platform) local chain can itself outlive its anchor's own two-day
-- reach (the anchor may be inherited from an already-old open session, or
-- the local chain itself may simply run long enough within one batch's read
-- window). Bucketing each event by
-- `floor(days_between(anchor_date, event_date) / 2)` splits the chain into
-- successive two-day epochs anchored on the SAME reference date throughout
-- (a deterministic function of two already-known dates, needing no running
-- state) — bucket 0 is the anchor's own reach; each later bucket's root is
-- the first (local-chain-relative) event to fall in it, exactly the
-- non-clock analogue of `sessionize`'s own day-aligned forced-root trick.
_bucketed AS (
    SELECT
        *,
        CAST(FLOOR(DATE_DIFF('day', CAST(_anchor_ts AS DATE), event_date) / 2.0) AS BIGINT) AS _epoch_bucket
    FROM _anchored
),
final AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        utm_campaign,
        CASE
            WHEN _epoch_bucket = 0 THEN _anchor_ts
            ELSE MIN(event_ts) OVER (PARTITION BY device_id, _local_root_ts, _epoch_bucket)
        END AS session_start_ts,
        CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_session_start END AS _seed_session_start,
        CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_event_count END AS _seed_event_count,
        CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_utm_campaign END AS _seed_utm_campaign
    FROM _bucketed
),
sessionized AS (
    SELECT
        *,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM final
),
-- Form B: same relation as `silver.sessions` — the partition_column
-- (session_start_date) is the session's own root day, and a session's events
-- land on that day or the next, never earlier, never more than one day
-- later (the bucketing above guarantees it: every row's `_epoch_bucket`
-- relative to its OWN assigned root is 0 by construction). This filter
-- declares that reach, so a run touching day D also rewrites its
-- skew-reached prior partitions — the same ±-day Form B write rebase
-- `silver.sessions` gets, composed with the self-edge's own `Ordered`
-- forcing (`docs/specs/incremental_shapes.md` §"Window independence and
-- self-referential models").
--
-- `aggregated` is a named CTE (rather than the model's own bare final
-- SELECT) purely so a `smelt.test` can target it via the test-local `#`
-- operator (`docs/specs/testing.md` §"CTE isolation") — a self-referential
-- model's own path and its self-reference share the same `smelt.<path>`
-- text, so a full-query test (`FROM smelt.silver.sessions_chained`) cannot
-- distinguish "the model under test" from "the self-reference inside it";
-- `#aggregated` sidesteps the collision by naming the model's external
-- dependencies (including the self-reference) as the mock boundary instead.
aggregated AS (
    SELECT
        CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
        device_id,
        session_start_ts,
        session_start_date,
        COALESCE(MAX(_seed_session_start), MIN(event_ts)) AS session_start,
        MAX(event_ts) AS session_end,
        COUNT(*) + COALESCE(MAX(_seed_event_count), 0) AS event_count,
        ANY_VALUE(platform) AS platform,
        -- Same 5-minute first-touch attribution as `silver.sessions`: an
        -- already-attributed campaign carried forward from an open session
        -- is kept (the window closed when that session first rooted);
        -- otherwise compute it fresh from this batch's own events.
        COALESCE(
            MAX(_seed_utm_campaign),
            ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
                WHERE utm_campaign IS NOT NULL
                    AND event_ts <= session_start_ts + INTERVAL '5 minutes'
            )
        ) AS utm_campaign
    FROM sessionized
    WHERE event_date
        BETWEEN session_start_date
            AND session_start_date + INTERVAL '1 day'
    GROUP BY device_id, session_start_ts, session_start_date
    HAVING MAX(event_ts) - COALESCE(MAX(_seed_session_start), MIN(event_ts)) < INTERVAL '2 days' -- root-anchored cap: explicit, checkable assertion
)
-- Explicit column list (not `SELECT *`): the batch-safety analyzer
-- (`smelt_logical::rules::incremental::detect`) reads the top-level SELECT's
-- own list to find the `partition_column`/`event_time_column` aliases and
-- classify each item; a wildcard item has no alias to classify and the
-- analyzer refuses the model outright (`could not be parsed`) — the actual
-- bound derivation (`smelt_logical::analysis::walk`) is unaffected either
-- way, since it walks the full (expanded) SQL text regardless of which
-- scope names which column.
SELECT
    session_id,
    device_id,
    session_start_ts,
    session_start_date,
    session_start,
    session_end,
    event_count,
    platform,
    utm_campaign
FROM aggregated
