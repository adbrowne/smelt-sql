-- Empirical validation for the incremental joins research note (§5.5).
-- Each property prints a row: (property, 0 = HOLDS, >0 = VIOLATED rows).
-- Multiset equality: |(L EXCEPT ALL R) ⊎ (R EXCEPT ALL L)| = 0.
--
-- The claim under test is which join shapes are safe to time-window (§5.3): a
-- filter on the driving fact's event-time commutes past a *window-invariant*
-- lookup (J1/J2), but silently loses rows when a second clock-bearing input is
-- independently windowed (J3/J4), and a fan-out join breaks the unique_key
-- contract the MERGE strategy depends on (J5).

-- ── Sources ──────────────────────────────────────────────────────────────────
-- Driving fact: an event stream partitioned on `created_at` (the event-time
-- column). user_id is a foreign key into the dimensions (50 distinct users).
CREATE TABLE events AS
  SELECT DATE '2024-01-01' + (i % 10)::INTEGER AS created_at,   -- Jan 1 .. Jan 10
         i                                     AS event_id,     -- unique per event
         (i % 50)                              AS user_id,
         (i % 5)                               AS category,
         (i % 100)::DOUBLE                     AS amount
  FROM range(0, 1000) t(i);

-- Static lookup dimension: category → name. No time column; window-invariant.
CREATE TABLE dim_static AS
  SELECT c AS category, 'cat_' || c::VARCHAR AS category_name
  FROM range(0, 5) t(c);

-- Timeseries dimension used as a lookup: every user was registered EARLY
-- (Jan 1), well before the events that reference them. It carries its own
-- `registered_at` clock, so `source_bounds` derives a bound for it and
-- `inject_source_filters` would window it — the §5.2 hazard.
CREATE TABLE dim_ts AS
  SELECT u                                     AS user_id,
         DATE '2024-01-01'                     AS registered_at,
         'country_' || (u % 3)::VARCHAR        AS country
  FROM range(0, 50) t(u);

-- Second fact for the fact ⋈ fact case: a separate event stream on the same
-- user_id key but its OWN independent clock (`created_at2`), spread across the
-- month and deliberately NOT aligned with the driving fact's day.
CREATE TABLE events2 AS
  SELECT DATE '2024-01-01' + ((i * 3) % 10)::INTEGER AS created_at2,
         (i % 50)                                    AS user_id,
         (i % 100)::DOUBLE                           AS amount2
  FROM range(0, 1000) t(i);

-- 1:N fan-out dimension: each category maps to THREE tag rows.
CREATE TABLE dim_tags AS
  SELECT c AS category, t AS tag
  FROM range(0, 5) cats(c), range(0, 3) tags(t);

-- Window w = [2024-01-03, 2024-01-07) for the single-window checks.
-- ─────────────────────────────────────────────────────────────────────────────

-- Property J1 (fact ⋈ static dim): a fact-side event-time filter commutes past a
-- static lookup.  σ_e( F ⋈ D )  ==  σ_e(F) ⋈ D .  Expect 0.
WITH
  lhs AS (SELECT * FROM (
            SELECT e.event_id, e.created_at AS event_time, e.user_id, d.category_name
            FROM events e JOIN dim_static d ON e.category = d.category) t
          WHERE event_time >= DATE '2024-01-03' AND event_time < DATE '2024-01-07'),
  rhs AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, d.category_name
          FROM (SELECT * FROM events
                WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07') e
          JOIN dim_static d ON e.category = d.category)
SELECT 'J1 fact_static_commutes' AS property,
       (SELECT count(*) FROM ((SELECT * FROM lhs EXCEPT ALL SELECT * FROM rhs)
                              UNION ALL
                              (SELECT * FROM rhs EXCEPT ALL SELECT * FROM lhs))) AS violations;

-- Property J2: incremental (two adjacent windows, fact filtered) over F ⋈ D_static
-- equals a full refresh.  Expect 0.
WITH
  body AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, d.category_name
           FROM events e JOIN dim_static d ON e.category = d.category),
  win1 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-01' AND event_time < DATE '2024-01-05'),
  win2 AS (SELECT * FROM body WHERE event_time >= DATE '2024-01-05' AND event_time < DATE '2024-01-11'),
  incremental AS (SELECT * FROM win1 UNION ALL SELECT * FROM win2),
  full_r AS (SELECT * FROM body)
SELECT 'J2 incremental_eq_full' AS property,
       (SELECT count(*) FROM ((SELECT * FROM incremental EXCEPT ALL SELECT * FROM full_r)
                              UNION ALL
                              (SELECT * FROM full_r EXCEPT ALL SELECT * FROM incremental))) AS violations;

-- Property J3 (HAZARD — timeseries dim independently windowed): the correct
-- result filters ONLY the driving fact and full-scans the dimension; the buggy
-- result (what inject_source_filters does today) also windows the dimension on
-- ITS OWN clock, dropping the early registration rows the join needs.  Expect >0.
WITH
  correct AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, d.country
              FROM (SELECT * FROM events
                    WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07') e
              JOIN dim_ts d ON e.user_id = d.user_id),
  buggy AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, d.country
            FROM (SELECT * FROM events
                  WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07') e
            JOIN (SELECT * FROM dim_ts
                  WHERE registered_at >= DATE '2024-01-03' AND registered_at < DATE '2024-01-07') d
              ON e.user_id = d.user_id)
SELECT 'J3 ts_dim_windowed (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM correct EXCEPT ALL SELECT * FROM buggy)
                              UNION ALL
                              (SELECT * FROM buggy EXCEPT ALL SELECT * FROM correct))) AS violations;

-- Property J4 (HAZARD — fact ⋈ fact on a non-partition key, both windowed):
-- a full refresh joins the whole of F2; the incremental approach windows BOTH
-- facts, dropping F2 counterparts outside the window.  Compares full (restricted
-- to F1's window) against both-windowed.  Expect >0.
WITH
  full_r AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, e2.amount2
             FROM events e JOIN events2 e2 ON e.user_id = e2.user_id
             WHERE e.created_at >= DATE '2024-01-03' AND e.created_at < DATE '2024-01-07'),
  both_windowed AS (SELECT e.event_id, e.created_at AS event_time, e.user_id, e2.amount2
                    FROM (SELECT * FROM events
                          WHERE created_at >= DATE '2024-01-03' AND created_at < DATE '2024-01-07') e
                    JOIN (SELECT * FROM events2
                          WHERE created_at2 >= DATE '2024-01-03' AND created_at2 < DATE '2024-01-07') e2
                      ON e.user_id = e2.user_id)
SELECT 'J4 fact_fact_both_windowed (expect >0)' AS property,
       (SELECT count(*) FROM ((SELECT * FROM full_r EXCEPT ALL SELECT * FROM both_windowed)
                              UNION ALL
                              (SELECT * FROM both_windowed EXCEPT ALL SELECT * FROM full_r))) AS violations;

-- Property J5 (HAZARD — OneToMany fan-out breaks the unique_key contract): the
-- MERGE strategy keys on unique_key (event_id), which must be unique in the
-- output. A 1:N join multiplies each event row, so event_id is no longer unique.
-- Reports the number of event_id values appearing more than once.  Expect >0.
SELECT 'J5 fanout_breaks_unique_key (expect >0)' AS property,
       (SELECT count(*) FROM (
          SELECT e.event_id
          FROM events e JOIN dim_tags d ON e.category = d.category
          WHERE e.created_at >= DATE '2024-01-03' AND e.created_at < DATE '2024-01-07'
          GROUP BY e.event_id
          HAVING count(*) > 1)) AS violations;
