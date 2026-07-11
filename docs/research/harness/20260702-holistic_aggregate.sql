-- Empirical validation for the §9.2 holistic-aggregate correction and the
-- §6.2 AT TIME ZONE whitelist correction (2026-07-02).
-- Each property prints a row: (property, 0 = HOLDS, >0 = VIOLATED rows).
-- Multiset equality: |(L EXCEPT ALL R) ∪ (R EXCEPT ALL L)| = 0.

-- ── Source ───────────────────────────────────────────────────────────────────
-- Four days of per-minute events with non-uniform values, so MEDIAN is a real
-- holistic computation (not degenerate).
CREATE TABLE src AS
  SELECT (TIMESTAMP '2024-01-01' + INTERVAL ((i % 4)) DAY
                                 + INTERVAL (i) MINUTE)       AS ts,
         (i * 37 % 101)::DOUBLE                               AS v
  FROM range(0, 400) t(i);

-- Property H1: a partition-aligned holistic aggregate is incremental ≡ full.
-- GROUP BY key = CAST(ts AS DATE) ⊇ partition_column, so every group lives in
-- exactly one window and is recomputed entire by the run that writes it —
-- MEDIAN per day over two adjacent windows equals the full-refresh MEDIAN.
-- This is the measurement behind withdrawing the proposed B7 gate (§9.2).
WITH
  full_r AS (SELECT CAST(ts AS DATE) AS d, MEDIAN(v) AS m FROM src GROUP BY 1),
  win1   AS (SELECT CAST(ts AS DATE) AS d, MEDIAN(v) AS m FROM src
             WHERE ts >= TIMESTAMP '2024-01-01' AND ts < TIMESTAMP '2024-01-03'
             GROUP BY 1),
  win2   AS (SELECT CAST(ts AS DATE) AS d, MEDIAN(v) AS m FROM src
             WHERE ts >= TIMESTAMP '2024-01-03' AND ts < TIMESTAMP '2024-01-05'
             GROUP BY 1),
  incremental AS (SELECT * FROM win1 UNION ALL SELECT * FROM win2)
SELECT 'H1 partition_aligned_median_eq_full' AS property,
       (SELECT count(*) FROM ((SELECT * FROM incremental EXCEPT ALL SELECT * FROM full_r)
                              UNION ALL
                              (SELECT * FROM full_r EXCEPT ALL SELECT * FROM incremental))) AS violations;

-- Property H2 (HAZARD): AT TIME ZONE with a named DST zone is NOT monotone.
-- Across the America/New_York fall-back (2024-11-03 02:00 EDT → 01:00 EST) the
-- instant→local mapping goes backward; count adjacent inversions over an
-- ordered instant grid. Expect violations > 0, proving the §6.2 whitelist may
-- admit fixed-offset zones only.
WITH
  grid AS (SELECT TIMESTAMPTZ '2024-11-03 04:00:00+00' + INTERVAL (i * 10) MINUTE AS ts
           FROM range(0, 24) t(i)),
  local_t AS (SELECT ts, ts AT TIME ZONE 'America/New_York' AS lt FROM grid)
SELECT 'H2 named_tz_dst_inversions (expect >0)' AS property,
       (SELECT count(*) FROM (
          SELECT lt, lag(lt) OVER (ORDER BY ts) AS prev_lt FROM local_t
        ) WHERE prev_lt IS NOT NULL AND lt < prev_lt) AS violations;
