<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/sessions.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Sessions and the cross-midnight backfill

<!-- PLACEHOLDER: intro prose — why any bounded sessionizer must cut. -->

## The sessions model

<!-- smelt-include: models/silver/sessions.sql -->
```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
WITH sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        utm_campaign,
        session_start_ts,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform,
    ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
        WHERE utm_campaign IS NOT NULL
            AND event_ts <= session_start_ts + INTERVAL '5 minutes'
    ) AS utm_campaign
FROM sessionized
WHERE event_date
    BETWEEN session_start_date
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
```

<!-- PLACEHOLDER: sessionize function summary + link; frontmatter notes. -->

## What a one-day run executes

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT
```

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-04-07' AND event_date < '2026-04-12') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
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
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-04-07' AND event_date < '2026-04-12') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
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
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    ```

<!-- PLACEHOLDER: window walkthrough. -->

## The cross-midnight rewrite

<!-- PLACEHOLDER: the 2026-05-04 00:03 event narrative. -->

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
COMMIT
```

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-05-04..2026-05-05`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-05-01' AND event_date < '2026-05-06') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
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
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-05-01' AND event_date < '2026-05-06') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
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
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
    COMMIT
    ```

## The alternative: let the session's own start decide

<!-- PLACEHOLDER: sessions_chained condensed treatment. -->

<!-- smelt-generate: @render=skeleton explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.sessions_chained" }
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT
```

??? example "Full emitted SQL — `smelt explain silver.sessions_chained --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
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
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
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
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
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
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
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
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.sessions_chained" }
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
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
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
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
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    ```

<!-- PLACEHOLDER: never-idle comparison table (pinned by e2e tests), trade-offs. -->
