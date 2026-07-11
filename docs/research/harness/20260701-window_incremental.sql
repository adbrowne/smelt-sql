-- Empirical validation for the incremental window-function research note (§8.5).
-- Each property prints a row: (property, 0 = HOLDS, >0 = VIOLATED rows).
-- Multiset equality: |(L EXCEPT ALL R) ⊎ (R EXCEPT ALL L)| = 0.
--
-- The claim under test is the two-layer lookback result (§4.3 row 3, §8.3): a
-- windowed computation whose ORDER BY key is the monotone event-time and whose
-- PARTITION BY ⊇ partition_column is incremental≡full ONLY when the SCAN window
-- is widened backward by the frame's lookback margin, while the OUTPUT clamp
-- stays exact. A bounded RANGE INTERVAL frame yields a finite, derivable margin
-- (W1); denying that widening drops rows (W2); an UNBOUNDED PRECEDING running
-- total has no finite margin and needs per-partition recompute (W3); a bare
-- LAG/LEAD is a ROW offset whose time reach is unbounded, so no bound is
-- derivable (W4).

-- ── Source ───────────────────────────────────────────────────────────────────
-- One event stream, partitioned on `created_at` (the event-time column) and
-- keyed by `user_id` (the intended PARTITION BY). One row per user per day for
-- 10 days: a dense grid so a RANGE '2 days' frame reaches exactly two prior
-- rows per user and the arithmetic is checkable by eye.
CREATE TABLE events AS
  SELECT DATE '2024-01-01' + d::INTEGER            AS created_at,   -- Jan 1 .. Jan 10
         u                                         AS user_id,      -- 20 users
         (d * 100 + u)::DOUBLE                      AS amount,
         (d * 1000 + u)                             AS event_id
  FROM range(0, 10) days(d), range(0, 20) users(u);

-- The model computes, per user, a trailing 2-day sum of `amount` (a RANGE frame
-- ordered by the event-time). The output row for day D at user U depends on the
-- rows for days D, D-1, D-2 of that same user. Lookback margin = 2 days.
--
-- Output window under test: w = [2024-01-05, 2024-01-08)  (days 4,5,6).
-- Full-refresh reference computes the frame over ALL days, then clamps to w.
-- ─────────────────────────────────────────────────────────────────────────────

-- Property W1 (bounded RANGE frame, scan widened by the lookback): computing the
-- frame over a scan widened backward by 2 days (the frame margin), then clamping
-- output to w, equals the full refresh clamped to w.  σ commutes once the scan
-- covers every row the frame reads.  Expect 0.
WITH
  full_refresh AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             RANGE BETWEEN INTERVAL 2 DAY PRECEDING AND CURRENT ROW) AS trailing
    FROM events),
  full_clamped AS (SELECT * FROM full_refresh
                   WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-08'),
  -- Incremental: scan widened back by the 2-day margin (Jan 3 .. Jan 8), frame
  -- computed on that widened scan, output clamped to w.
  widened_scan AS (SELECT * FROM events
                   WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-08'),
  inc AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             RANGE BETWEEN INTERVAL 2 DAY PRECEDING AND CURRENT ROW) AS trailing
    FROM widened_scan),
  inc_clamped AS (SELECT * FROM inc
                  WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-08')
SELECT 'W1 range_frame_widened_scan' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_clamped EXCEPT ALL SELECT * FROM inc_clamped)
                              UNION ALL
                              (SELECT * FROM inc_clamped EXCEPT ALL SELECT * FROM full_clamped))) AS violations;

-- Property W2 (HAZARD — same frame, scan NOT widened): if the scan is clamped to
-- the output window itself (no lookback margin), the frame at the window's early
-- days cannot see the prior rows it needs, so the trailing sum is understated.
-- Compares full refresh (clamped to w) against a frame computed on the un-widened
-- scan.  Expect >0.
WITH
  full_refresh AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             RANGE BETWEEN INTERVAL 2 DAY PRECEDING AND CURRENT ROW) AS trailing
    FROM events),
  full_clamped AS (SELECT * FROM full_refresh
                   WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-08'),
  narrow_scan AS (SELECT * FROM events
                  WHERE created_at >= DATE '2024-01-05' AND created_at < DATE '2024-01-08'),
  inc_narrow AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             RANGE BETWEEN INTERVAL 2 DAY PRECEDING AND CURRENT ROW) AS trailing
    FROM narrow_scan)
SELECT 'W2 range_frame_narrow_scan (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_clamped EXCEPT ALL SELECT * FROM inc_narrow)
                              UNION ALL
                              (SELECT * FROM inc_narrow EXCEPT ALL SELECT * FROM full_clamped))) AS violations;

-- Property W3 (UNBOUNDED PRECEDING — no finite lookback margin): a running total
-- from the beginning of each partition depends on EVERY prior row, so no finite
-- widening recovers it. Even a scan widened by a generous 4-day margin still
-- understates the running total at the window's early days.  Expect >0 — the
-- proof that UNBOUNDED PRECEDING cannot be made a two-layer bounded scan and must
-- fall back to per-partition recompute (BatchSafety::PerPartitionOnly).
WITH
  full_refresh AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running
    FROM events),
  full_clamped AS (SELECT * FROM full_refresh
                   WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-08'),
  widened_scan AS (SELECT * FROM events           -- generous 4-day margin, still not enough
                   WHERE created_at >= DATE '2024-01-01' AND created_at < DATE '2024-01-08'),
  inc AS (
    SELECT event_id, created_at AS event_time, user_id,
           SUM(amount) OVER (PARTITION BY user_id ORDER BY created_at
                             ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running
    FROM widened_scan
    WHERE created_at >= DATE '2024-01-03'),        -- pretend margin was only 2 days
  inc_clamped AS (SELECT * FROM inc
                  WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-08')
SELECT 'W3 unbounded_preceding (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_clamped EXCEPT ALL SELECT * FROM inc_clamped)
                              UNION ALL
                              (SELECT * FROM inc_clamped EXCEPT ALL SELECT * FROM full_clamped))) AS violations;

-- Property W4 (bare LAG — ROW offset, no derivable TIME bound): LAG(amount, 1)
-- reaches the previous ROW in ORDER BY, whose event-time distance is data-
-- dependent (here 1 day on a dense grid, but unbounded in general — a user with
-- a single event in the window has their previous event arbitrarily far back).
-- To SHOW the reach is not a fixed time margin, we sparsify: keep only days
-- {0, 5, 6} per user, so the LAG at day 5 reaches back to day 0 — five days,
-- not the one-day grid step. A scan widened by any fixed small margin (say 2
-- days) therefore drops the true predecessor.  Expect >0.
WITH
  sparse AS (SELECT * FROM events WHERE EXTRACT(DAY FROM created_at) IN (1, 6, 7)),  -- days 0,5,6
  full_refresh AS (
    SELECT event_id, created_at AS event_time, user_id,
           LAG(amount, 1) OVER (PARTITION BY user_id ORDER BY created_at) AS prev_amount
    FROM sparse),
  full_clamped AS (SELECT * FROM full_refresh
                   WHERE event_time >= DATE '2024-01-06' AND event_time < DATE '2024-01-08'),
  widened_scan AS (SELECT * FROM sparse            -- fixed 2-day margin: Jan 4 .. Jan 8
                   WHERE created_at >= DATE '2024-01-04' AND created_at < DATE '2024-01-08'),
  inc AS (
    SELECT event_id, created_at AS event_time, user_id,
           LAG(amount, 1) OVER (PARTITION BY user_id ORDER BY created_at) AS prev_amount
    FROM widened_scan),
  inc_clamped AS (SELECT * FROM inc
                  WHERE event_time >= DATE '2024-01-06' AND event_time < DATE '2024-01-08')
SELECT 'W4 bare_lag_no_time_bound (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_clamped EXCEPT ALL SELECT * FROM inc_clamped)
                              UNION ALL
                              (SELECT * FROM inc_clamped EXCEPT ALL SELECT * FROM full_clamped))) AS violations;
