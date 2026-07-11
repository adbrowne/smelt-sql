-- Empirical validation for the incremental subquery-in-FROM research note (§3.5).
-- Each property prints a row: (property, 0 = HOLDS, >0 = VIOLATED rows).
-- Multiset equality: |(L EXCEPT ALL R) ⊎ (R EXCEPT ALL L)| = 0.
--
-- The claim under test is the commutation of the window predicate with the
-- subquery body (§3.2): σ_event_time( Q(R) ) == Q( σ_event_time(R) ). Where it
-- holds, the filter proven safe at the outer SELECT can equivalently be pushed
-- to the source (Part 4, §4.3) — Q1 and Q4 double as that push-to-source check.

-- ── Source ─────────────────────────────────────────────────────────────────
-- A single event stream partitioned on `created_at` (the event-time column).
CREATE TABLE events AS
  SELECT DATE '2024-01-01' + (i % 10)::INTEGER AS created_at,
         i                                     AS user_id,
         (i % 5)                               AS category,
         (i % 100)::DOUBLE                      AS amount,
         (i % 2 = 0)                            AS active
  FROM range(0, 1000) t(i);

-- Window w = [2024-01-03, 2024-01-07) for the single-window checks.
-- ─────────────────────────────────────────────────────────────────────────────

-- Property Q1 (transparent body): filtering the OUTPUT of a project/filter/rename
-- subquery equals pushing the same filter to the SOURCE.
--   σ_e( π σ'(R) )  ==  π σ'( σ_e(R) )
-- Also validates Part 4 §4.3 row 1: for a transparent body the outer clamp and
-- the source-level filter coincide.
WITH
  lhs AS (SELECT * FROM (SELECT user_id, created_at AS event_time, amount
                         FROM events WHERE active) t
          WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  rhs AS (SELECT user_id, created_at AS event_time, amount
          FROM events
          WHERE active AND created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07')
SELECT 'Q1 transparent_pushdown' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT ALL SELECT * FROM rhs)
                              UNION ALL
                              (SELECT * FROM rhs EXCEPT ALL SELECT * FROM lhs))) AS violations;

-- Property Q2: incremental (two adjacent windows) over a transparent subquery
-- equals a full refresh.
--   [2024-01-01,2024-01-05) ⊎ [2024-01-05,2024-01-11)  ==  full
WITH
  body AS (SELECT user_id, created_at AS event_time, amount FROM events WHERE active),
  win1 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-01' AND event_time < DATE '2024-01-05'),
  win2 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-11'),
  incremental AS (SELECT * FROM win1 UNION ALL SELECT * FROM win2),
  full_r AS (SELECT * FROM body)
SELECT 'Q2 incremental_eq_full' AS property,
       (SELECT count(*) FROM ((SELECT * FROM incremental EXCEPT ALL SELECT * FROM full_r)
                              UNION ALL
                              (SELECT * FROM full_r EXCEPT ALL SELECT * FROM incremental))) AS violations;

-- Property Q3: a derived table and the equivalent CTE produce identical results
-- (confirms §3.3 — the two spellings denote the same query, so the gate that
-- rejects one while allowing the other is keying on syntax, not semantics).
WITH
  derived AS (SELECT * FROM (SELECT user_id, created_at AS event_time FROM events) t
              WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  cte AS (
    WITH inner_q AS (SELECT user_id, created_at AS event_time FROM events)
    SELECT * FROM inner_q
    WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07')
SELECT 'Q3 derived_eq_cte' AS property,
       (SELECT count(*) FROM ((SELECT * FROM derived EXCEPT ALL SELECT * FROM cte)
                              UNION ALL
                              (SELECT * FROM cte EXCEPT ALL SELECT * FROM derived))) AS violations;

-- Property Q4 (group-aligned aggregating body): when the body's GROUP BY key
-- ⊇ partition_column (here it IS the day), filtering the OUTPUT equals pushing
-- the filter BELOW the aggregate to the source.
--   σ_day( γ_day(R) )  ==  γ_day( σ_day(R) )
-- Validates §3.2 row 3 and Part 4 §4.3 row 2 (below-aggregate pushdown).
WITH
  lhs AS (SELECT * FROM (SELECT created_at AS event_time, COUNT(*) AS n, SUM(amount) AS total
                         FROM events GROUP BY created_at) t
          WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  rhs AS (SELECT created_at AS event_time, COUNT(*) AS n, SUM(amount) AS total
          FROM events
          WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07'
          GROUP BY created_at)
SELECT 'Q4 groupaligned_pushdown' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT ALL SELECT * FROM rhs)
                              UNION ALL
                              (SELECT * FROM rhs EXCEPT ALL SELECT * FROM lhs))) AS violations;

-- Property Q5a (HAZARD — LIMIT body): a LIMIT does NOT commute with the window
-- predicate. Clamping the output of a top-N body (σ after LIMIT) differs from
-- pushing the filter to the source (LIMIT after σ). Expect violations > 0.
WITH
  outer_clamp AS (SELECT * FROM (SELECT user_id, created_at AS event_time
                                 FROM events ORDER BY user_id LIMIT 50) t
                  WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  pushed AS (SELECT user_id, created_at AS event_time
             FROM events
             WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07'
             ORDER BY user_id LIMIT 50)
SELECT 'Q5a limit_not_pushable (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM outer_clamp EXCEPT ALL SELECT * FROM pushed)
                              UNION ALL
                              (SELECT * FROM pushed EXCEPT ALL SELECT * FROM outer_clamp))) AS violations;

-- Property Q5b (HAZARD — cross-window window frame): a running total over an
-- unbounded frame depends on rows outside the window. Computing it over the full
-- history then filtering (outer clamp) differs from filtering first then
-- computing it (pushed). Expect violations > 0 — this is why such bodies stay
-- PerPartitionOnly / need a lookback rather than a naive source push.
WITH
  outer_clamp AS (SELECT * FROM (SELECT user_id, created_at AS event_time,
                                        SUM(amount) OVER (ORDER BY created_at, user_id) AS running
                                 FROM events) t
                  WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  pushed AS (SELECT user_id, created_at AS event_time,
                    SUM(amount) OVER (ORDER BY created_at, user_id) AS running
             FROM events
             WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07')
SELECT 'Q5b window_frame_not_pushable (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM outer_clamp EXCEPT ALL SELECT * FROM pushed)
                              UNION ALL
                              (SELECT * FROM pushed EXCEPT ALL SELECT * FROM outer_clamp))) AS violations;
