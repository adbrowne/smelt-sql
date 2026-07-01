-- Empirical validation for the incremental UNION-ALL research note.
-- Each property prints a row: (property, 0 = HOLDS, >0 = VIOLATED rows).
-- Multiset equality: |(L EXCEPT ALL R) ∪ (R EXCEPT ALL L)| = 0.

-- ── Sources ────────────────────────────────────────────────────────────────
-- big: a high-volume event stream; small: a low-volume event source (case 1).
CREATE TABLE big AS
  SELECT DATE '2024-01-01' + (i % 10)::INTEGER AS event_time,
         i AS user_id, 'web' AS kind
  FROM range(0, 1000) t(i);

CREATE TABLE small AS
  SELECT DATE '2024-01-01' + (i % 10)::INTEGER AS event_time,
         i AS user_id, 'seed' AS kind
  FROM range(0, 7) t(i);

-- small_null: a static/lookup branch that projects NULL in the event_time slot
-- (case 2 — the hazard). Same column shape so it is UNION-compatible.
CREATE TABLE small_null AS
  SELECT CAST(NULL AS DATE) AS event_time, 999 AS user_id, 'dim' AS kind;

-- The run window used for the single-window distributivity check.
-- w = [2024-01-03, 2024-01-07)
-- ─────────────────────────────────────────────────────────────────────────────

-- Property 1: filter distributes over UNION ALL.
--   σ(big ⊎ small)  ==  σ(big) ⊎ σ(small)
WITH
  lhs AS (SELECT * FROM (SELECT * FROM big UNION ALL SELECT * FROM small)
          WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  rhs AS (SELECT * FROM big  WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'
          UNION ALL
          SELECT * FROM small WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07')
SELECT 'P1 union_all_distributes' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT ALL SELECT * FROM rhs)
                              UNION ALL
                              (SELECT * FROM rhs EXCEPT ALL SELECT * FROM lhs))) AS violations;

-- Property 2: incremental (two adjacent windows) == full refresh, over UNION ALL.
--   [2024-01-01,2024-01-05) ⊎ [2024-01-05,2024-01-11)  ==  full
WITH
  body AS (SELECT * FROM big UNION ALL SELECT * FROM small),
  win1 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-01' AND event_time < DATE '2024-01-05'),
  win2 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-11'),
  incremental AS (SELECT * FROM win1 UNION ALL SELECT * FROM win2),
  full_r AS (SELECT * FROM body)
SELECT 'P2 incremental_eq_full' AS property,
       (SELECT count(*) FROM ((SELECT * FROM incremental EXCEPT ALL SELECT * FROM full_r)
                              UNION ALL
                              (SELECT * FROM full_r EXCEPT ALL SELECT * FROM incremental))) AS violations;

-- Property 3 (HAZARD): a branch projecting NULL event_time appears in full refresh
-- but is silently dropped from every windowed run. Expect violations > 0, proving
-- the "must be independently partitionable" precondition is load-bearing.
WITH
  body AS (SELECT * FROM big UNION ALL SELECT * FROM small_null),
  windowed AS (SELECT * FROM body
               WHERE event_time >= DATE '2024-01-01' AND event_time < DATE '2024-01-11'),
  full_r AS (SELECT * FROM body)
SELECT 'P3 null_eventtime_hazard (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_r EXCEPT ALL SELECT * FROM windowed)
                              UNION ALL
                              (SELECT * FROM windowed EXCEPT ALL SELECT * FROM full_r))) AS violations;

-- Property 4: filter distributes over UNION (distinct), INTERSECT, EXCEPT too.
-- Use two overlapping event sets so the set semantics actually bite.
CREATE TABLE ea AS SELECT DATE '2024-01-01' + (i % 8)::INTEGER AS event_time, (i % 20) AS user_id FROM range(0,200) t(i);
CREATE TABLE eb AS SELECT DATE '2024-01-01' + (i % 8)::INTEGER AS event_time, (i % 30) AS user_id FROM range(0,200) t(i);

-- UNION (distinct)
WITH
  lhs AS (SELECT * FROM (SELECT * FROM ea UNION SELECT * FROM eb)
          WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'),
  rhs AS ((SELECT * FROM ea WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06')
          UNION
          (SELECT * FROM eb WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'))
SELECT 'P4a union_distinct_distributes' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT SELECT * FROM rhs)
                              UNION (SELECT * FROM rhs EXCEPT SELECT * FROM lhs))) AS violations;

-- INTERSECT
WITH
  lhs AS (SELECT * FROM (SELECT * FROM ea INTERSECT SELECT * FROM eb)
          WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'),
  rhs AS ((SELECT * FROM ea WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06')
          INTERSECT
          (SELECT * FROM eb WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'))
SELECT 'P4b intersect_distributes' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT SELECT * FROM rhs)
                              UNION (SELECT * FROM rhs EXCEPT SELECT * FROM lhs))) AS violations;

-- EXCEPT
WITH
  lhs AS (SELECT * FROM (SELECT * FROM ea EXCEPT SELECT * FROM eb)
          WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'),
  rhs AS ((SELECT * FROM ea WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06')
          EXCEPT
          (SELECT * FROM eb WHERE event_time >= DATE '2024-01-02' AND event_time < DATE '2024-01-06'))
SELECT 'P4c except_distributes' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT SELECT * FROM rhs)
                              UNION (SELECT * FROM rhs EXCEPT SELECT * FROM lhs))) AS violations;
